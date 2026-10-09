//! Stage 0 protocol proof of concept: drives a Yeelight Obsid (Chameleon2)
//! strip step by step so each protocol assumption can be checked on hardware.
//! Run `obsid-poc help` for usage.

use std::net::Ipv4Addr;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use obsid_ambilight::discovery::{self, DiscoveredDevice, DiscoveryOptions};
use obsid_ambilight::protocol::device::{Chameleon2, ExitStrategy};
use obsid_ambilight::protocol::packets::Call;
use obsid_ambilight::protocol::session::SessionConfig;
use obsid_ambilight::protocol::transport::UdpTransport;
use obsid_ambilight::protocol::{ProtocolError, Rgb};
use tracing::Level;

const USAGE: &str = "\
obsid-poc — Yeelight Obsid (Chameleon2) protocol proof of concept

USAGE:
    obsid-poc [OPTIONS] <COMMAND> [ARGS]

OPTIONS:
    --ip <IPv4>          Use this address instead of discovery
    --id <device-id>     Pick the device with this id from discovery
    --leds <N>           Number of LEDs for direct mode (default 60, max 120)
    -v / -vv             Debug / trace logging (trace shows packets, tokens redacted)

COMMANDS:
    discover [--all] [--timeout-ms N]   List Chameleon2 strips (--all: every Yeelight)
    session                             Handshake + keep-alive, print stats
    on | off                            set_power
    bright <1-100>                      set_bright
    color <#RRGGBB>                     set_rgb
    static <#RRGGBB> <1-100>            power on + colour + brightness
    raw <method> [json-params]          Send any method and print the reply
    direct [--seconds S] [--fps F] [--pattern P]
                                        Stream frames; P = rainbow|chase|segments|ends
    exit-test <STRATEGY> [--stream-seconds S] [--hold-seconds H] [--color #hex] [--bright N]
                                        Stream, leave direct mode, then go silent for H s
                                        STRATEGY = deactivate | set-rgb | power-cycle | expire[:secs]
    sequence [--color #hex] [--bright N]
                                        Full Stage 0 scenario with pauses
    watch [--seconds S]                 Diagnostics: probe the strip every second without a
                                        session; log reachability and advertised state changes
";

struct Opts {
    ip: Option<Ipv4Addr>,
    id: Option<String>,
    leds: usize,
    verbosity: u8,
    command: String,
    args: Vec<String>,
}

fn main() -> ExitCode {
    let opts = match parse_args(std::env::args().skip(1).collect()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let level = match opts.verbosity {
        0 => Level::INFO,
        1 => Level::DEBUG,
        _ => Level::TRACE,
    };
    tracing_subscriber::fmt().with_max_level(level).with_target(false).with_writer(std::io::stderr).init();

    match run(&opts) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: Vec<String>) -> Result<Opts, String> {
    let mut opts = Opts { ip: None, id: None, leds: 60, verbosity: 0, command: String::new(), args: Vec::new() };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--ip" => {
                let v = it.next().ok_or("--ip needs a value")?;
                opts.ip = Some(v.parse().map_err(|_| format!("bad IPv4 address: {v}"))?);
            }
            "--id" => opts.id = Some(it.next().ok_or("--id needs a value")?),
            "--leds" => {
                let v = it.next().ok_or("--leds needs a value")?;
                opts.leds = v.parse().map_err(|_| format!("bad LED count: {v}"))?;
            }
            "-v" => opts.verbosity = 1,
            "-vv" => opts.verbosity = 2,
            "-h" | "--help" if opts.command.is_empty() => opts.command = "help".into(),
            _ if opts.command.is_empty() && !a.starts_with('-') => opts.command = a,
            _ if !opts.command.is_empty() => opts.args.push(a),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    if opts.command.is_empty() {
        return Err("missing command".into());
    }
    Ok(opts)
}

type AnyResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn run(o: &Opts) -> AnyResult {
    match o.command.as_str() {
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        "discover" => cmd_discover(&o.args),
        "session" => {
            let dev = connect(o)?;
            let s = dev.session().stats();
            println!(
                "session OK: sent={} received={} ignored={} handshakes={}",
                s.sent, s.received, s.ignored, s.handshakes
            );
            Ok(())
        }
        "on" | "off" => Ok(connect(o)?.set_power(o.command == "on")?),
        "bright" => Ok(connect(o)?.set_brightness(parse_bright(arg(&o.args, 0, "brightness")?)?)?),
        "color" => Ok(connect(o)?.set_color(parse_color(arg(&o.args, 0, "colour")?)?)?),
        "static" => {
            let color = parse_color(arg(&o.args, 0, "colour")?)?;
            let bright = parse_bright(arg(&o.args, 1, "brightness")?)?;
            Ok(connect(o)?.apply_static(color, bright)?)
        }
        "raw" => cmd_raw(o),
        "direct" => cmd_direct(o),
        "exit-test" => cmd_exit_test(o),
        "sequence" => cmd_sequence(o),
        "watch" => cmd_watch(o),
        other => Err(format!("unknown command {other}").into()),
    }
}

fn cmd_discover(args: &[String]) -> AnyResult {
    let all = args.iter().any(|a| a == "--all");
    let timeout_ms = flag(args, "--timeout-ms")?.map(|v| v.parse::<u64>()).transpose()?.unwrap_or(3000);
    match discovery::interfaces::local_interfaces() {
        Ok(list) if !list.is_empty() => {
            println!("searching for {timeout_ms} ms via:");
            for a in list {
                println!("  {} ({}/{})", a.name, a.ip, a.prefix_len);
            }
        }
        Ok(_) => println!("no usable adapters; searching via the default route for {timeout_ms} ms"),
        Err(e) => println!("adapter enumeration failed ({e}); searching via the default route"),
    }
    let opts = DiscoveryOptions { timeout: Duration::from_millis(timeout_ms), ..Default::default() };
    let devices: Vec<_> = discovery::discover(&opts)?.into_iter().filter(|d| all || d.is_chameleon2()).collect();
    if devices.is_empty() {
        println!("no {}devices answered", if all { "" } else { "Chameleon2 " });
    }
    for d in &devices {
        print_device(d);
    }
    Ok(())
}

fn print_device(d: &DiscoveredDevice) {
    println!("- {} ({})", d.display_name(), d.model);
    println!("    id:        {}", d.id);
    println!("    ip:        {}", d.ip);
    println!("    location:  {}", d.location);
    println!("    fw_ver:    {}", d.fw_ver.as_deref().unwrap_or("?"));
    println!("    power:     {}", d.power.as_deref().unwrap_or("?"));
    println!("    rgbic udp: {}", d.supports_rgbic_udp());
    println!("    support:   {}", d.support.join(" "));
}

fn cmd_raw(o: &Opts) -> AnyResult {
    let method = arg(&o.args, 0, "method")?;
    let params = match o.args.get(1) {
        Some(p) => serde_json::from_str(p).map_err(|e| format!("params must be JSON: {e}"))?,
        None => serde_json::json!([]),
    };
    let mut dev = connect(o)?;
    match dev.session_mut().request(&Call::raw(method, params)) {
        Ok(result) => println!("result: {}", serde_json::Value::Array(result)),
        Err(ProtocolError::Device { code, message }) => println!("device error {code}: {message}"),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn cmd_direct(o: &Opts) -> AnyResult {
    let duration = flag(&o.args, "--seconds")?.map(parse_secs).transpose()?.unwrap_or(Duration::from_secs(10));
    let fps: u32 = flag(&o.args, "--fps")?.map(str::parse).transpose()?.unwrap_or(20);
    let pattern = flag(&o.args, "--pattern")?.unwrap_or("rainbow");
    let mut dev = connect(o)?;
    dev.enter_direct()?;
    stream(&mut dev, pattern, duration, fps)?;
    println!("streaming stopped; the strip is left in direct mode (use exit-test to study leaving it)");
    Ok(())
}

fn cmd_exit_test(o: &Opts) -> AnyResult {
    let strategy = parse_strategy(arg(&o.args, 0, "strategy")?)?;
    let stream_for = flag(&o.args, "--stream-seconds")?.map(parse_secs).transpose()?.unwrap_or(Duration::from_secs(5));
    let hold_s: u64 = flag(&o.args, "--hold-seconds")?.map(str::parse).transpose()?.unwrap_or(25);
    let color = flag(&o.args, "--color")?.map(parse_color).transpose()?.unwrap_or(Rgb::new(0xF5, 0xD4, 0xA1));
    let bright = flag(&o.args, "--bright")?.map(parse_bright).transpose()?.unwrap_or(40);

    let mut dev = connect(o)?;
    dev.apply_static(Rgb::new(0, 0, 255), 60)?;
    step(&mut dev, "Baseline: strip should be solid BLUE now (static mode).", 3)?;
    dev.enter_direct()?;
    stream(&mut dev, "rainbow", stream_for, 20)?;
    println!("frames stopped; leaving direct mode with {strategy:?}, target {color} @ {bright}%");
    let t = Instant::now();
    match dev.exit_direct(strategy, color, bright) {
        Ok(()) => println!("exit sequence acknowledged by device in {:?}", t.elapsed()),
        Err(e) => println!("exit sequence FAILED after {:?}: {e}", t.elapsed()),
    }
    print_props(&mut dev, "right after exit");
    println!(
        "Now silent for {hold_s} s (no frames, no keep-alive) so the UDP session expires.\n\
         OBSERVE: does the strip show {color} at once, and does it STAY that way?"
    );
    for left in (1..=hold_s).rev() {
        if left % 5 == 0 {
            println!("  {left} s left...");
        }
        thread::sleep(Duration::from_secs(1));
    }
    println!("Silence over. Reading state through a new session:");
    print_props(&mut connect(o)?, "after silence");
    println!("Now check that Apple Home can still change colour/brightness/power of the strip.");
    Ok(())
}

fn print_props(dev: &mut Chameleon2<UdpTransport>, when: &str) {
    const PROPS: [&str; 4] = ["power", "bright", "rgb", "color_mode"];
    match dev.session_mut().request(&Call::raw("get_prop", serde_json::json!(PROPS))) {
        Ok(v) => {
            let shown: Vec<String> = PROPS.iter().zip(&v).map(|(k, v)| format!("{k}={v}")).collect();
            println!("  state {when}: {}", shown.join(" "));
        }
        Err(e) => println!("  get_prop {when}: {e}"),
    }
}

/// Polls the strip with search requests (no UDP control session, so the probe
/// itself does not interfere with other controllers) and logs when it
/// drops off the network or its advertised state changes.
fn cmd_watch(o: &Opts) -> AnyResult {
    let seconds: u64 = flag(&o.args, "--seconds")?.map(str::parse).transpose()?.unwrap_or(300);
    let ip = match o.ip {
        Some(ip) => ip,
        None => pick_device(o.id.as_deref())?.ip,
    };
    println!("watching {ip} for {seconds} s (Ctrl+C to stop); times are UTC");
    let start = Instant::now();
    let mut last_state: Option<String> = None;
    let mut down_since: Option<Instant> = None;
    let mut last_print = start;
    while start.elapsed() < Duration::from_secs(seconds) {
        let tick = Instant::now();
        // fw 36 ignores unicast M-SEARCH, so use a short multicast search.
        let opts = DiscoveryOptions {
            timeout: Duration::from_millis(900),
            resend_every: Duration::from_millis(900),
            broadcast: false,
        };
        let reply = discovery::discover(&opts)?.into_iter().find(|d| d.ip == ip);
        let stamp = utc_stamp();
        match reply {
            Some(d) => {
                if let Some(since) = down_since.take() {
                    println!("{stamp} BACK after {:.1} s unreachable", since.elapsed().as_secs_f32());
                }
                let state = format!(
                    "power={} {}",
                    d.power.as_deref().unwrap_or("?"),
                    d.state.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ")
                );
                if last_state.as_ref() != Some(&state) {
                    println!("{stamp} state {state}");
                    last_state = Some(state);
                    last_print = Instant::now();
                } else if last_print.elapsed() >= Duration::from_secs(15) {
                    println!("{stamp} ok");
                    last_print = Instant::now();
                }
            }
            None if down_since.is_none() => {
                println!("{stamp} NO REPLY — strip unreachable");
                down_since = Some(tick);
            }
            None => {}
        }
        if let Some(rest) = Duration::from_secs(1).checked_sub(tick.elapsed()) {
            thread::sleep(rest);
        }
    }
    Ok(())
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("{:02}:{:02}:{:02}", secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

fn cmd_sequence(o: &Opts) -> AnyResult {
    let color = flag(&o.args, "--color")?.map(parse_color).transpose()?.unwrap_or(Rgb::new(0xF5, 0xD4, 0xA1));
    let bright = flag(&o.args, "--bright")?.map(parse_bright).transpose()?.unwrap_or(40);
    let mut dev = connect(o)?;
    println!("1/7 power on");
    dev.set_power(true)?;
    step(&mut dev, "Strip should be ON.", 3)?;
    println!("2/7 static colour {color} @ {bright}%");
    dev.set_color(color)?;
    dev.set_brightness(bright)?;
    step(&mut dev, "Strip should show the static colour.", 4)?;
    println!("3/7 power off");
    dev.set_power(false)?;
    step(&mut dev, "Strip should be OFF.", 3)?;
    println!("4/7 power on + direct mode");
    dev.set_power(true)?;
    dev.set_brightness(100)?;
    dev.enter_direct()?;
    println!("5/7 streaming 'ends' pattern: first LED red, last LED green, every 10th white");
    stream(&mut dev, "ends", Duration::from_secs(8), 20)?;
    println!("6/7 streaming 'chase' for 8 s: note the direction of travel");
    stream(&mut dev, "chase", Duration::from_secs(8), 20)?;
    println!("7/7 back to static via set-rgb strategy");
    dev.exit_direct(ExitStrategy::SetRgb, color, bright)?;
    println!("Done. Run `exit-test` for each strategy to check the colour persists after the session ends.");
    Ok(())
}

/// Streams a test pattern while keeping the session alive.
fn stream(dev: &mut Chameleon2<UdpTransport>, pattern: &str, duration: Duration, fps: u32) -> AnyResult {
    let fps = fps.clamp(1, 60);
    let period = Duration::from_secs(1) / fps;
    let n = dev.led_count();
    let start = Instant::now();
    let mut next = start;
    let mut frames = 0u64;
    let mut late = 0u64;
    while start.elapsed() < duration {
        let now = Instant::now();
        // A single unanswered keep-alive is not fatal: the session only ends
        // after `max_missed_keep_alives` (then tick returns NoSession).
        match dev.tick(now) {
            Err(ProtocolError::Timeout) => eprintln!("  keep-alive unanswered, continuing"),
            other => other?,
        }
        let t = now.duration_since(start).as_secs_f32();
        let frame = render(pattern, n, t)?;
        dev.send_frame(&frame)?;
        frames += 1;
        next += period;
        match next.checked_duration_since(Instant::now()) {
            Some(wait) => thread::sleep(wait),
            None => {
                late += 1;
                next = Instant::now();
            }
        }
    }
    let secs = start.elapsed().as_secs_f64();
    let s = dev.session().stats();
    println!(
        "  {frames} frames in {secs:.1} s ({:.1} fps, {late} late); replies received={} last_error={:?}",
        frames as f64 / secs,
        s.received,
        s.last_error
    );
    Ok(())
}

fn render(pattern: &str, n: usize, t: f32) -> AnyResult<Vec<Rgb>> {
    let frame = match pattern {
        "rainbow" => (0..n).map(|i| hsv(i as f32 / n as f32 + t * 0.2, 1.0, 1.0)).collect(),
        "chase" => {
            let head = (t * 10.0) as usize % n;
            (0..n).map(|i| if i == head { Rgb::new(255, 255, 255) } else { Rgb::new(0, 0, 24) }).collect()
        }
        "segments" => {
            const C: [Rgb; 6] = [
                Rgb::new(255, 0, 0),
                Rgb::new(255, 128, 0),
                Rgb::new(255, 255, 0),
                Rgb::new(0, 255, 0),
                Rgb::new(0, 0, 255),
                Rgb::new(160, 0, 255),
            ];
            (0..n).map(|i| C[(i / 10) % C.len()]).collect()
        }
        "ends" => (0..n)
            .map(|i| match i {
                0 => Rgb::new(255, 0, 0),
                _ if i == n - 1 => Rgb::new(0, 255, 0),
                _ if i % 10 == 0 => Rgb::new(255, 255, 255),
                _ => Rgb::new(0, 0, 0),
            })
            .collect(),
        other => return Err(format!("unknown pattern {other}").into()),
    };
    Ok(frame)
}

fn hsv(h: f32, s: f32, v: f32) -> Rgb {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |f: f32| ((f + m) * 255.0).round() as u8;
    Rgb::new(to(r), to(g), to(b))
}

fn connect(o: &Opts) -> AnyResult<Chameleon2<UdpTransport>> {
    let ip = match o.ip {
        Some(ip) => ip,
        None => pick_device(o.id.as_deref())?.ip,
    };
    println!("connecting to {ip}:{}", obsid_ambilight::protocol::CONTROL_PORT);
    Ok(Chameleon2::connect(ip, o.leds, SessionConfig::default())?)
}

fn pick_device(id: Option<&str>) -> AnyResult<DiscoveredDevice> {
    let found: Vec<_> =
        discovery::discover(&DiscoveryOptions::default())?.into_iter().filter(|d| d.is_chameleon2()).collect();
    let mut matching: Vec<_> = found.into_iter().filter(|d| id.is_none_or(|id| d.id == id)).collect();
    match matching.len() {
        0 => Err("no matching Chameleon2 found; try `discover --all` or pass --ip".into()),
        1 => Ok(matching.remove(0)),
        _ => Err("several Chameleon2 strips found; pass --id or --ip".into()),
    }
}

/// Pauses for observation while keeping the session alive.
fn step(dev: &mut Chameleon2<UdpTransport>, msg: &str, secs: u64) -> AnyResult {
    println!("   -> {msg}");
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        let now = Instant::now();
        if now >= until {
            return Ok(());
        }
        match dev.tick(now) {
            Err(ProtocolError::Timeout) => eprintln!("  keep-alive unanswered, continuing"),
            other => other?,
        }
        thread::sleep((until - now).min(Duration::from_millis(250)));
    }
}

fn arg<'a>(args: &'a [String], i: usize, what: &str) -> Result<&'a str, String> {
    args.get(i).map(String::as_str).ok_or_else(|| format!("missing {what}"))
}

fn flag<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>, String> {
    match args.iter().position(|a| a == name) {
        Some(i) => args.get(i + 1).map(|v| Some(v.as_str())).ok_or_else(|| format!("{name} needs a value")),
        None => Ok(None),
    }
}

fn parse_color(s: &str) -> Result<Rgb, String> {
    Rgb::from_hex(s).ok_or_else(|| format!("bad colour {s}, expected #RRGGBB"))
}

/// Seconds as a finite, non-negative number up to one hour; validated before
/// any command reaches the device.
fn parse_secs(s: &str) -> Result<Duration, String> {
    s.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && (0.0..=3600.0).contains(v))
        .map(Duration::from_secs_f64)
        .ok_or_else(|| format!("bad duration {s}, expected 0-3600 seconds"))
}

fn parse_bright(s: &str) -> Result<u8, String> {
    s.parse::<u8>().ok().filter(|b| (1..=100).contains(b)).ok_or_else(|| format!("bad brightness {s}, expected 1-100"))
}

fn parse_strategy(s: &str) -> Result<ExitStrategy, String> {
    Ok(match s {
        "set-rgb" => ExitStrategy::SetRgb,
        "power-cycle" => ExitStrategy::PowerCycle,
        "deactivate" => ExitStrategy::Deactivate,
        "expire" => ExitStrategy::ExpireSession { wait: Duration::from_secs(15) },
        _ => match s.strip_prefix("expire:").and_then(|v| v.parse::<u64>().ok()) {
            Some(secs) => ExitStrategy::ExpireSession { wait: Duration::from_secs(secs) },
            None => return Err(format!("unknown strategy {s}")),
        },
    })
}

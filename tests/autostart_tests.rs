use std::path::Path;

use obsid_ambilight::platform::autostart::{self, command_line};

/// A throw-away `…\Run` key per test under HKCU\Software\ObsidAmbilightTest;
/// the whole per-test subtree is deleted afterwards.
struct TestKey(String, String);

impl TestKey {
    fn new(name: &str) -> Self {
        let root = format!(r"Software\ObsidAmbilightTest\{name}-{}", std::process::id());
        let _ = autostart::delete_tree(&root);
        Self(format!(r"{root}\Run"), root)
    }
}

impl Drop for TestKey {
    fn drop(&mut self) {
        let _ = autostart::delete_tree(&self.1);
    }
}

#[test]
fn command_line_quotes_the_path() {
    assert_eq!(command_line(Path::new(r"C:\Program Files\Obsid\a.exe")), r#""C:\Program Files\Obsid\a.exe""#);
}

#[test]
fn values_are_written_read_and_removed() {
    let key = TestKey::new("rw");
    assert_eq!(autostart::read(&key.0, "App").unwrap(), None, "missing key reads as none");
    autostart::write(&key.0, "App", r#""C:\x\a.exe""#).unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap().as_deref(), Some(r#""C:\x\a.exe""#));
    autostart::remove(&key.0, "App").unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap(), None);
    autostart::remove(&key.0, "App").unwrap();
}

#[test]
fn sync_adds_and_removes_the_entry() {
    let key = TestKey::new("sync");
    let exe = Path::new(r"C:\Apps\obsid-ambilight.exe");
    autostart::sync(&key.0, "App", true, exe).unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap(), Some(command_line(exe)));
    autostart::sync(&key.0, "App", false, exe).unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap(), None);
}

#[test]
fn sync_updates_a_stale_path() {
    let key = TestKey::new("stale");
    autostart::write(&key.0, "App", r#""C:\Old\obsid-ambilight.exe""#).unwrap();
    let exe = Path::new(r"C:\New\obsid-ambilight.exe");
    autostart::sync(&key.0, "App", true, exe).unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap(), Some(command_line(exe)));
}

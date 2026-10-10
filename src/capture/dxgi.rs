//! DXGI Desktop Duplication on the monitor's own adapter. Frames are scaled
//! down on the GPU with mipmaps; only an image at most [`MAX_SMALL_WIDTH`]
//! pixels wide is read back.

use windows::Win32::Foundation::{E_FAIL, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_RESOURCE_MISC_GENERATE_MIPS, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource,
};
use windows::core::{Error, Interface};

use super::outputs::{self, FoundOutput};
use super::{Grabber, GrabberInfo, display};
use crate::ambilight::color;
use crate::ambilight::image::Image;

/// Widest image read back from the GPU.
pub const MAX_SMALL_WIDTH: u32 = 256;
/// SDR white when the display does not report it (scRGB 1.0).
const DEFAULT_WHITE_NITS: f32 = 80.0;

/// Mip level whose width first fits in [`MAX_SMALL_WIDTH`].
pub fn mip_level(width: u32) -> u32 {
    let mut level = 0;
    while (width >> level) > MAX_SMALL_WIDTH {
        level += 1;
    }
    level
}

pub struct DxgiGrabber {
    info: GrabberInfo,
    context: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    mips: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    staging: ID3D11Texture2D,
    level: u32,
    small: (u32, u32),
    _device: ID3D11Device,
}

fn texture(device: &ID3D11Device, desc: &D3D11_TEXTURE2D_DESC) -> windows::core::Result<ID3D11Texture2D> {
    let mut tex = None;
    // SAFETY: valid descriptor, no initial data.
    unsafe { device.CreateTexture2D(desc, None, Some(&mut tex))? };
    tex.ok_or_else(|| Error::from(E_FAIL))
}

fn desc(width: u32, height: u32, mips: u32, format: DXGI_FORMAT) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mips,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: 0,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    }
}

impl DxgiGrabber {
    /// Opens the monitor named `monitor` (empty = primary); an unknown name
    /// falls back to the primary monitor with `info().fallback` set.
    pub fn open(monitor: &str) -> Result<Self, String> {
        let all = outputs::enumerate().map_err(|e| format!("cannot list monitors: {}", e.message()))?;
        let wanted = all.iter().position(|o| !monitor.is_empty() && o.name == monitor);
        let fallback = !monitor.is_empty() && wanted.is_none();
        let index = wanted
            .or_else(|| all.iter().position(FoundOutput::is_primary))
            .or(if all.is_empty() { None } else { Some(0) })
            .ok_or("no monitor is attached to the desktop")?;
        let found = &all[index];
        Self::create(found, fallback).map_err(|e| format!("cannot capture {}: {}", found.name, e.message()))
    }

    fn create(found: &FoundOutput, fallback: bool) -> windows::core::Result<Self> {
        let (mut device, mut context) = (None, None);
        // SAFETY: the adapter outlives the call; out-pointers are valid.
        unsafe {
            D3D11CreateDevice(
                &found.adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device: ID3D11Device = device.ok_or_else(|| Error::from(E_FAIL))?;
        let context: ID3D11DeviceContext = context.ok_or_else(|| Error::from(E_FAIL))?;
        // SAFETY: COM calls on interfaces owned here.
        let dup = unsafe {
            match found.output.cast::<IDXGIOutput5>() {
                Ok(o5) => {
                    o5.DuplicateOutput1(&device, 0, &[DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_B8G8R8A8_UNORM])?
                }
                // Windows 10 before 1703: SDR only.
                Err(_) => found.output.cast::<IDXGIOutput1>()?.DuplicateOutput(&device)?,
            }
        };
        // SAFETY: plain getter.
        let mode = unsafe { dup.GetDesc() }.ModeDesc;
        let (w, h, format) = (mode.Width, mode.Height, mode.Format);
        let hdr = format == DXGI_FORMAT_R16G16B16A16_FLOAT;
        let level = mip_level(w);
        let small = ((w >> level).max(1), (h >> level).max(1));

        let mips = texture(
            &device,
            &D3D11_TEXTURE2D_DESC {
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
                ..desc(w, h, level + 1, format)
            },
        )?;
        let mut srv = None;
        // SAFETY: `mips` is a shader resource of this device.
        unsafe { device.CreateShaderResourceView(&mips, None, Some(&mut srv))? };
        let srv = srv.ok_or_else(|| Error::from(E_FAIL))?;
        let staging = texture(
            &device,
            &D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..desc(small.0, small.1, 1, format)
            },
        )?;
        let sdr_white_nits =
            if hdr { display::sdr_white_nits(&found.name).unwrap_or(DEFAULT_WHITE_NITS) } else { DEFAULT_WHITE_NITS };
        let info = GrabberInfo {
            monitor: found.name.clone(),
            adapter: found.adapter_name.clone(),
            width: w,
            height: h,
            hdr,
            sdr_white_nits,
            fallback,
        };
        Ok(Self { info, context, dup, mips, srv, staging, level, small, _device: device })
    }

    /// Copies the acquired frame into the mip chain and reads the small level back.
    fn read(
        &self,
        frame: &DXGI_OUTDUPL_FRAME_INFO,
        resource: Option<IDXGIResource>,
    ) -> windows::core::Result<Option<Image>> {
        // Only the mouse pointer moved: the desktop image is unchanged.
        if frame.LastPresentTime == 0 {
            return Ok(None);
        }
        let Some(resource) = resource else { return Ok(None) };
        let desktop: ID3D11Texture2D = resource.cast()?;
        let (w, h) = (self.small.0 as usize, self.small.1 as usize);
        // SAFETY: all resources belong to this device; the mapped pointer is
        // read only within RowPitch × h bytes and unmapped before returning.
        unsafe {
            self.context.CopySubresourceRegion(&self.mips, 0, 0, 0, 0, &desktop, 0, None);
            self.context.GenerateMips(&self.srv);
            self.context.CopySubresourceRegion(&self.staging, 0, 0, 0, 0, &self.mips, self.level, None);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let base = mapped.pData as *const u8;
            let bytes = if self.info.hdr { 8 } else { 4 };
            let mut pixels = Vec::with_capacity(w * h);
            for y in 0..h {
                let row = std::slice::from_raw_parts(base.add(y * mapped.RowPitch as usize), w * bytes);
                if self.info.hdr {
                    pixels.extend(row.as_chunks::<8>().0.iter().map(|p| {
                        let ch = |i: usize| color::f16_to_f32(u16::from_le_bytes([p[i], p[i + 1]]));
                        color::hdr_to_srgb([ch(0), ch(2), ch(4)], self.info.sdr_white_nits)
                    }));
                } else {
                    pixels.extend(
                        row.as_chunks::<4>()
                            .0
                            .iter()
                            .map(|p| [f32::from(p[2]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[0]) / 255.0]),
                    );
                }
            }
            self.context.Unmap(&self.staging, 0);
            Ok(Some(Image::new(w, h, pixels)))
        }
    }
}

impl Grabber for DxgiGrabber {
    fn info(&self) -> &GrabberInfo {
        &self.info
    }

    fn grab(&mut self) -> Result<Option<Image>, String> {
        let mut frame = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: out-pointers are valid for the call.
        match unsafe { self.dup.AcquireNextFrame(0, &mut frame, &mut resource) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            // ACCESS_LOST (mode/HDR change, full screen switch), ACCESS_DENIED
            // (secure desktop) and everything else: reopen.
            Err(e) => return Err(format!("capture lost: {}", e.message())),
        }
        let result = self.read(&frame, resource);
        // Every acquired frame must be released, whatever happened above.
        // SAFETY: a frame is currently acquired.
        let _ = unsafe { self.dup.ReleaseFrame() };
        result.map_err(|e| format!("capture failed: {}", e.message()))
    }
}

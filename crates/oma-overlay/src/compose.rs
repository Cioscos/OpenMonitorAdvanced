//! The drawing surface of the overlay window (spec §5.1): a D3D11 device, a
//! DXGI swapchain for composition (premultiplied alpha, flip model) shown
//! by DirectComposition, and a Direct2D device context on its back buffer.
//! Nothing goes through `UpdateLayeredWindow`.
//!
//! On a lost device every call fails with one of the codes of
//! [`needs_recreate`]: the window loop drops the `Compositor` and builds a
//! new one at the next frame.

use windows::core::{Interface, Result, HRESULT};
use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1,
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
    DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_PRESENT,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

/// The largest side of the swapchain, in pixels (the D3D11 texture limit).
const MAX_SIDE: u32 = 16384;

/// Whether `hr` means the device is gone and everything must be rebuilt.
pub(crate) fn needs_recreate(hr: HRESULT) -> bool {
    hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET || hr == D2DERR_RECREATE_TARGET
}

fn clamp_side(v: u32) -> u32 {
    v.clamp(1, MAX_SIDE)
}

const TRANSPARENT: D2D1_COLOR_F = D2D1_COLOR_F {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// Device, swapchain, composition tree and Direct2D context of one window.
/// Every object is owned here (COM references), so dropping the
/// `Compositor` releases the whole chain; the window outlives it.
pub struct Compositor {
    swapchain: IDXGISwapChain1,
    dc: ID2D1DeviceContext,
    /// The back buffer as the context's target; `None` while resizing.
    target: Option<ID2D1Bitmap1>,
    size: (u32, u32),
    /// Between `begin` and `end_and_present`.
    drawing: bool,
    // Kept alive for the lifetime of the composition: the target binds the
    // visual to the window, the visual shows the swapchain.
    _dcomp_target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    _dcomp: IDCompositionDevice,
    _d2d_device: ID2D1Device,
    _d3d: ID3D11Device,
}

impl Compositor {
    /// Builds the chain for `hwnd` with a `width × height` swapchain.
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let (width, height) = (clamp_side(width), clamp_side(height));
        let mut device: Option<ID3D11Device> = None;
        // SAFETY: valid out-pointer to a local; no adapter, no software
        // module, default feature levels; the device is returned owned.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?
        };
        let d3d = device.ok_or_else(|| windows::core::Error::from(DXGI_ERROR_DEVICE_REMOVED))?;
        let dxgi_device: IDXGIDevice = d3d.cast()?;
        // SAFETY: no flags; the factory is returned owned.
        let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };
        // SAFETY: `desc` lives on the stack for the call; the device is a
        // live D3D11 device; no output restriction.
        let swapchain =
            unsafe { factory.CreateSwapChainForComposition(&dxgi_device, &desc, None)? };

        // SAFETY: plain factory creation; the factory, device and context
        // are returned owned and used from this thread only.
        let (d2d_device, dc) = unsafe {
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d.CreateDevice(&dxgi_device)?;
            let dc = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            (d2d_device, dc)
        };
        let target = bind_target(&dc, &swapchain)?;

        // SAFETY: `hwnd` is a live window of this thread; the visual takes a
        // reference to the swapchain, the target to the visual, and all
        // three are owned by the returned `Compositor`.
        let (dcomp, dcomp_target, visual) = unsafe {
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi_device)?;
            let dcomp_target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swapchain)?;
            dcomp_target.SetRoot(&visual)?;
            dcomp.Commit()?;
            (dcomp, dcomp_target, visual)
        };
        Ok(Self {
            swapchain,
            dc,
            target: Some(target),
            size: (width, height),
            drawing: false,
            _dcomp_target: dcomp_target,
            _visual: visual,
            _dcomp: dcomp,
            _d2d_device: d2d_device,
            _d3d: d3d,
        })
    }

    /// The swapchain size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Resizes the swapchain; nothing to do at the same size.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (clamp_side(width), clamp_side(height));
        if (width, height) == self.size && self.target.is_some() {
            return Ok(());
        }
        self.end_open_draw();
        // `ResizeBuffers` needs every reference to the back buffers gone:
        // the context lets go of its target, and we drop ours.
        // SAFETY: the context is ours and not drawing (ended above).
        unsafe { self.dc.SetTarget(None) };
        self.target = None;
        // SAFETY: no outstanding back-buffer references (see above); 0 and
        // `DXGI_FORMAT_UNKNOWN` keep the count and the format.
        unsafe {
            self.swapchain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?
        };
        self.size = (width, height);
        self.target = Some(bind_target(&self.dc, &self.swapchain)?);
        Ok(())
    }

    /// Starts a frame: the context, cleared to transparent, in pixels.
    pub fn begin(&mut self) -> &ID2D1DeviceContext {
        // A frame abandoned after an error is closed first.
        self.end_open_draw();
        // SAFETY: the context has its target (set in `new`/`resize`) and is
        // used from this thread only; the colour is a constant.
        unsafe {
            self.dc.BeginDraw();
            self.dc.Clear(Some(&TRANSPARENT));
        }
        self.drawing = true;
        &self.dc
    }

    /// Ends the frame and presents it at the next vertical blank.
    pub fn end_and_present(&mut self) -> Result<()> {
        if self.drawing {
            self.drawing = false;
            // SAFETY: closes the `BeginDraw` of `begin`; no tag out-pointers.
            unsafe { self.dc.EndDraw(None, None)? };
        }
        // SAFETY: the swapchain is ours; sync interval 1, no flags. Status
        // codes such as `DXGI_STATUS_OCCLUDED` are successes.
        unsafe { self.swapchain.Present(1, DXGI_PRESENT(0)).ok() }
    }

    fn end_open_draw(&mut self) {
        if self.drawing {
            self.drawing = false;
            // SAFETY: closes an open `BeginDraw`; the result belongs to a
            // frame already given up.
            let _ = unsafe { self.dc.EndDraw(None, None) };
        }
    }
}

/// Wraps the swapchain's current back buffer as the context's target, with
/// 96 DPI so that one Direct2D unit is one physical pixel.
fn bind_target(dc: &ID2D1DeviceContext, swapchain: &IDXGISwapChain1) -> Result<ID2D1Bitmap1> {
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        ..Default::default()
    };
    // SAFETY: buffer 0 of a flip-model swapchain is the one to draw; `props`
    // lives on the stack for the call; the bitmap holds its own reference to
    // the surface, which `resize` releases before `ResizeBuffers`.
    unsafe {
        let surface: IDXGISurface = swapchain.GetBuffer(0)?;
        let bitmap = dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
        dc.SetTarget(&bitmap);
        dc.SetDpi(96.0, 96.0);
        Ok(bitmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{E_FAIL, E_OUTOFMEMORY};
    use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
    use windows::Win32::Graphics::Direct2D::D2D1_ROUNDED_RECT;

    #[test]
    fn needs_recreate_codes() {
        assert!(needs_recreate(DXGI_ERROR_DEVICE_REMOVED));
        assert!(needs_recreate(DXGI_ERROR_DEVICE_RESET));
        assert!(needs_recreate(D2DERR_RECREATE_TARGET));
        assert!(!needs_recreate(E_FAIL));
        assert!(!needs_recreate(E_OUTOFMEMORY));
        assert!(!needs_recreate(HRESULT(0)));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn creates_composition_swapchain_and_presents() {
        // The window stays hidden: nothing reaches the user's screen.
        let hwnd = crate::window::create().expect("overlay window");
        let mut gfx = Compositor::new(hwnd, 64, 32).expect("compositor");
        assert_eq!(gfx.size(), (64, 32));
        let dc = gfx.begin();
        // SAFETY: drawing on our context between `begin` and `end_and_present`.
        unsafe {
            let brush = dc
                .CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.35,
                    },
                    None,
                )
                .expect("brush");
            let rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: 64.0,
                    bottom: 32.0,
                },
                radiusX: 4.0,
                radiusY: 4.0,
            };
            dc.FillRoundedRectangle(&rect, &brush);
        }
        gfx.end_and_present().expect("present");
        gfx.resize(128, 48).expect("resize");
        assert_eq!(gfx.size(), (128, 48));
        gfx.begin();
        gfx.end_and_present().expect("present after resize");
        drop(gfx);
        crate::window::destroy(hwnd);
    }
}

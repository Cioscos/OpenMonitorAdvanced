//! The D3D11 device of one adapter, chosen by LUID (plan DG4), with the few resource
//! helpers the GPU kernels share.

use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Buffer, ID3D11ComputeShader, ID3D11Device, ID3D11DeviceContext,
    ID3D11ShaderResourceView, ID3D11UnorderedAccessView, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_UNORDERED_ACCESS, D3D11_BOX, D3D11_BUFFER_DESC,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, D3D11_SDK_VERSION, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter3, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
};

/// DXGI HRESULTs of a GPU that Windows or the driver has reset.
const DEVICE_REMOVED: i32 = 0x887A_0005_u32 as i32;
const DEVICE_HUNG: i32 = 0x887A_0006_u32 as i32;
const DEVICE_RESET: i32 = 0x887A_0007_u32 as i32;
const DRIVER_INTERNAL_ERROR: i32 = 0x887A_0020_u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000E_u32 as i32;
const E_FAIL: i32 = 0x8000_4005_u32 as i32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuError {
    /// No hardware adapter has the LUID.
    NotFound,
    /// A call failed with this HRESULT.
    Create(i32),
    /// The device is gone: the HRESULT of `GetDeviceRemovedReason`, or of the failed call
    /// when the device gives no reason.
    Lost(u32),
    /// A submission did not finish within 1 s.
    Hung,
    /// The GPU clock changed during every try of a timing, so there is no GPU time.
    TimingDisjoint,
    OutOfMemory,
    /// The GPU's output differs from the CPU reference in the sample check of a phase
    /// (plan DG5): a defect of the implementation or the driver, not of the GPU.
    ReferenceInvalid,
}

/// Classifies a failed HRESULT; `removed_reason` is asked only for a lost device.
pub fn map_hresult(hr: i32, removed_reason: impl FnOnce() -> u32) -> GpuError {
    match hr {
        DEVICE_REMOVED | DEVICE_HUNG | DEVICE_RESET | DRIVER_INTERNAL_ERROR => {
            match removed_reason() {
                0 => GpuError::Lost(hr as u32),
                reason => GpuError::Lost(reason),
            }
        }
        E_OUTOFMEMORY => GpuError::OutOfMemory,
        _ => GpuError::Create(hr),
    }
}

/// The HRESULT of `GetDeviceRemovedReason`: 0 while the device works.
pub(crate) fn removed_reason(device: &ID3D11Device) -> u32 {
    // SAFETY: a method without arguments on a live device.
    match unsafe { device.GetDeviceRemovedReason() } {
        Ok(()) => 0,
        Err(e) => e.code().0 as u32,
    }
}

/// A failed call on `device`, classified by [`map_hresult`].
pub(crate) fn gpu_error(device: &ID3D11Device, e: &windows::core::Error) -> GpuError {
    map_hresult(e.code().0, || removed_reason(device))
}

/// The size of `buffer` in bytes.
fn byte_width(buffer: &ID3D11Buffer) -> u32 {
    let mut desc = D3D11_BUFFER_DESC::default();
    // SAFETY: `desc` is a live local of the type the method fills.
    unsafe { buffer.GetDesc(&mut desc) };
    desc.ByteWidth
}

/// `(HighPart << 32) | LowPart`, as in `oma-win`.
fn luid_to_u64(luid: LUID) -> u64 {
    ((luid.HighPart as u32 as u64) << 32) | luid.LowPart as u64
}

/// A D3D11 device (feature level 11_0) on one hardware adapter. A clone shares the device.
#[derive(Clone)]
pub struct GpuDevice {
    adapter: IDXGIAdapter3,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dedicated: u64,
}

impl GpuDevice {
    /// Opens the hardware adapter whose packed LUID is `luid`; software adapters never match.
    pub fn open(luid: u64) -> Result<GpuDevice, GpuError> {
        let failed = |e: windows::core::Error| map_hresult(e.code().0, || 0);
        // SAFETY: no preconditions; the factory is released when dropped.
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(failed)?;
        let mut index = 0;
        let (adapter, desc) = loop {
            // SAFETY: an index past the last adapter gives DXGI_ERROR_NOT_FOUND.
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
                return Err(GpuError::NotFound);
            };
            index += 1;
            // SAFETY: a method without arguments on a live adapter.
            let desc = unsafe { adapter.GetDesc1() }.map_err(failed)?;
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0
                && luid_to_u64(desc.AdapterLuid) == luid
            {
                break (adapter, desc);
            }
        };
        let (mut device, mut context) = (None, None);
        // SAFETY: a hardware adapter with the UNKNOWN driver type (required with an
        // adapter), no software module, one feature level; the out pointers are live locals.
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(failed)?;
        Ok(GpuDevice {
            adapter: adapter.cast().map_err(failed)?,
            device: device.ok_or(GpuError::Create(E_FAIL))?,
            context: context.ok_or(GpuError::Create(E_FAIL))?,
            dedicated: desc.DedicatedVideoMemory as u64,
        })
    }

    /// `(budget, usage)` of the local segment of node 0.
    pub fn video_memory(&self) -> Result<(u64, u64), GpuError> {
        let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        // SAFETY: node 0 exists on every adapter; `info` is a live local of the right type.
        unsafe {
            self.adapter
                .QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info)
        }
        .map_err(|e| self.error(&e))?;
        Ok((info.Budget, info.CurrentUsage))
    }

    pub fn dedicated_bytes(&self) -> u64 {
        self.dedicated
    }

    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    pub fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }

    /// A failed call on this device, classified by [`map_hresult`].
    pub fn error(&self, e: &windows::core::Error) -> GpuError {
        gpu_error(&self.device, e)
    }

    pub fn compute_shader(&self, bytecode: &[u8]) -> Result<ID3D11ComputeShader, GpuError> {
        let mut shader = None;
        // SAFETY: `bytecode` is fxc output for cs_5_0; no class linkage; live out pointer.
        unsafe {
            self.device
                .CreateComputeShader(bytecode, None, Some(&mut shader))
        }
        .map_err(|e| self.error(&e))?;
        shader.ok_or(GpuError::Create(E_FAIL))
    }

    /// A 16-byte constant buffer, filled with [`GpuDevice::set_constants`].
    pub fn constant_buffer(&self) -> Result<ID3D11Buffer, GpuError> {
        self.buffer(&D3D11_BUFFER_DESC {
            ByteWidth: 16,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        })
    }

    /// Fills a constant buffer from [`GpuDevice::constant_buffer`].
    pub fn set_constants(&self, buffer: &ID3D11Buffer, values: [u32; 4]) {
        self.write_words(buffer, &values);
    }

    /// Replaces the whole of `buffer`, a DEFAULT buffer of this device.
    ///
    /// # Panics
    /// When `buffer` is not exactly `4 * words.len()` bytes.
    pub fn write_words(&self, buffer: &ID3D11Buffer, words: &[u32]) {
        assert_eq!(
            byte_width(buffer) as usize,
            size_of_val(words),
            "write_words must fill the whole buffer"
        );
        // SAFETY: the update without a box reads `ByteWidth` bytes from the source, which
        // is exactly `words` (asserted); the pitches are ignored for buffers.
        unsafe {
            self.context
                .UpdateSubresource(buffer, 0, None, words.as_ptr().cast(), 0, 0)
        }
    }

    /// Replaces element `index` of `buffer`, a DEFAULT structured buffer of this device with
    /// 16-byte elements. D3D11 silently drops a partial update of a structured element, so
    /// a whole element it is.
    ///
    /// # Panics
    /// When the element is past the end of `buffer`.
    pub fn write_element(&self, buffer: &ID3D11Buffer, index: u32, element: [u32; 4]) {
        assert!(
            (u64::from(index) + 1) * 16 <= u64::from(byte_width(buffer)),
            "write_element past the end of the buffer"
        );
        let region = D3D11_BOX {
            left: index * 16,
            right: index * 16 + 16,
            bottom: 1,
            back: 1,
            ..Default::default()
        };
        // SAFETY: the box covers 16 bytes inside the buffer (asserted), read from `element`.
        unsafe {
            self.context
                .UpdateSubresource(buffer, 0, Some(&region), element.as_ptr().cast(), 0, 0)
        }
    }

    /// A structured buffer of `bytes` (a multiple of `stride`) with a UAV and an SRV.
    pub fn structured_buffer(
        &self,
        bytes: u32,
        stride: u32,
    ) -> Result<
        (
            ID3D11Buffer,
            ID3D11UnorderedAccessView,
            ID3D11ShaderResourceView,
        ),
        GpuError,
    > {
        let buffer = self.buffer(&D3D11_BUFFER_DESC {
            ByteWidth: bytes,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_UNORDERED_ACCESS.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: stride,
            ..Default::default()
        })?;
        let (mut uav, mut srv) = (None, None);
        // SAFETY: a buffer of this device bound for both views; the default descriptions
        // view the whole structured buffer; live out pointers.
        unsafe {
            self.device
                .CreateUnorderedAccessView(&buffer, None, Some(&mut uav))
                .and_then(|()| {
                    self.device
                        .CreateShaderResourceView(&buffer, None, Some(&mut srv))
                })
        }
        .map_err(|e| self.error(&e))?;
        Ok((
            buffer,
            uav.ok_or(GpuError::Create(E_FAIL))?,
            srv.ok_or(GpuError::Create(E_FAIL))?,
        ))
    }

    /// Copies the first `bytes` of `src` to the CPU. `Map` waits for the GPU without the
    /// 1 s limit of the submissions, so call it after `Submit::finish`.
    ///
    /// # Panics
    /// When `src` is smaller than `bytes`.
    pub fn read_buffer(&self, src: &ID3D11Buffer, bytes: u32) -> Result<Vec<u8>, GpuError> {
        assert!(
            bytes <= byte_width(src),
            "read_buffer past the end of the buffer"
        );
        // ponytail: a staging buffer per read; keep one if reads ever get frequent.
        let staging = self.buffer(&D3D11_BUFFER_DESC {
            ByteWidth: bytes,
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        })?;
        assert_eq!(byte_width(&staging), bytes);
        let region = D3D11_BOX {
            right: bytes,
            bottom: 1,
            back: 1,
            ..Default::default()
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: `src` holds at least `bytes` and `staging` exactly `bytes` (asserted), so
        // the box fits both; the mapped pointer is valid for `bytes` until `Unmap`, and the
        // slice is copied out before it.
        unsafe {
            self.context
                .CopySubresourceRegion(&staging, 0, 0, 0, 0, src, 0, Some(&region));
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| self.error(&e))?;
            let data =
                std::slice::from_raw_parts(mapped.pData as *const u8, bytes as usize).to_vec();
            self.context.Unmap(&staging, 0);
            Ok(data)
        }
    }

    fn buffer(&self, desc: &D3D11_BUFFER_DESC) -> Result<ID3D11Buffer, GpuError> {
        let mut buffer = None;
        // SAFETY: a complete description, no initial data, live out pointer.
        unsafe { self.device.CreateBuffer(desc, None, Some(&mut buffer)) }
            .map_err(|e| self.error(&e))?;
        buffer.ok_or(GpuError::Create(E_FAIL))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_hresults_map_to_device_lost() {
        for hr in [
            DEVICE_REMOVED,
            DEVICE_HUNG,
            DEVICE_RESET,
            DRIVER_INTERNAL_ERROR,
        ] {
            assert_eq!(map_hresult(hr, || 0x887A_0006), GpuError::Lost(0x887A_0006));
            // Without a reason (S_OK), the failed call's HRESULT stands in.
            assert_eq!(map_hresult(hr, || 0), GpuError::Lost(hr as u32));
        }
    }

    #[test]
    fn other_hresults_are_create_errors() {
        let never = || -> u32 { panic!("the removed reason is only for lost devices") };
        let invalid_arg = 0x8007_0057_u32 as i32;
        assert_eq!(
            map_hresult(invalid_arg, never),
            GpuError::Create(invalid_arg)
        );
        assert_eq!(map_hresult(E_OUTOFMEMORY, never), GpuError::OutOfMemory);
    }

    /// The first hardware GPU; these tests only create buffers, they run no GPU work.
    fn first_gpu() -> GpuDevice {
        let adapter = oma_win::gpu::stress_adapters().into_iter().next();
        GpuDevice::open(adapter.expect("no hardware GPU").luid).unwrap()
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    #[should_panic(expected = "write_words must fill the whole buffer")]
    fn write_words_rejects_a_short_slice() {
        let gpu = first_gpu();
        let (counters, _, _) = gpu.structured_buffer(32, 4).unwrap();
        gpu.write_words(&counters, &[0; 4]);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    #[should_panic(expected = "read_buffer past the end of the buffer")]
    fn read_buffer_rejects_a_read_past_the_end() {
        let gpu = first_gpu();
        let (counters, _, _) = gpu.structured_buffer(32, 4).unwrap();
        let _ = gpu.read_buffer(&counters, 64);
    }
}

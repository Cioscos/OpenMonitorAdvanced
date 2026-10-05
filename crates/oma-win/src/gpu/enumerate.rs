//! Lists the hardware GPUs: DXGI for identity and VRAM size, DXCore for
//! integrated vs discrete, D3DKMT for the PCI address.

use std::collections::HashMap;
use std::mem::size_of;

use oma_core::provider::ProviderError;
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::DXCore::{
    IDXCoreAdapter, IDXCoreAdapterFactory, IDXCoreAdapterList, InstanceLuid, IsIntegrated,
    DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE,
    DXGI_ERROR_NOT_FOUND,
};

use super::adapter::{Adapter, PciAddress};
use super::d3dkmt::KmtAdapter;

/// ADAPTERTYPE bit 5: the integrated half of a hybrid (iGPU + dGPU) system.
const HYBRID_INTEGRATED: u32 = 1 << 5;
/// ADAPTERADDRESS bus number of adapters without a PCI location
/// (e.g. Microsoft Basic Render Driver).
const NO_BUS: u32 = u32::MAX;

/// Packs a LUID as `(HighPart << 32) | LowPart`, the key shared by every GPU layer.
pub(crate) fn luid_to_u64(luid: LUID) -> u64 {
    ((luid.HighPart as u32 as u64) << 32) | luid.LowPart as u64
}

/// Inverse of [`luid_to_u64`].
pub(crate) fn u64_to_luid(value: u64) -> LUID {
    LUID {
        LowPart: value as u32,
        HighPart: (value >> 32) as u32 as i32,
    }
}

/// `false` for DXGI software adapters (Microsoft Basic Render Driver, WARP).
pub(crate) fn is_hardware(dxgi_flags: u32) -> bool {
    dxgi_flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0
}

/// PCI location from D3DKMT ADAPTERADDRESS; `None` for adapters that are not
/// on a PCI bus (bus 0xFFFFFFFF) or report impossible device/function numbers.
pub(crate) fn pci_from_address(bus: u32, device: u32, function: u32) -> Option<PciAddress> {
    (bus != NO_BUS && device < 32 && function < 8).then_some(PciAddress {
        bus,
        device,
        function,
    })
}

/// DXCore `IsIntegrated` when the driver reports it, else the D3DKMT
/// ADAPTERTYPE HybridIntegrated bit, else discrete (decision D4).
pub(crate) fn integrated_from(dxcore: Option<bool>, adapter_type_bits: Option<u32>) -> bool {
    dxcore.unwrap_or_else(|| adapter_type_bits.is_some_and(|bits| bits & HYBRID_INTEGRATED != 0))
}

/// DXGI descriptions are NUL-padded UTF-16 and sometimes end with spaces.
pub(crate) fn adapter_name(description: &[u16]) -> String {
    let len = description
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(description.len());
    String::from_utf16_lossy(&description[..len])
        .trim()
        .to_owned()
}

/// Hardware GPUs in DXGI order (the primary display adapter first).
pub(crate) fn enumerate() -> Result<Vec<Adapter>, ProviderError> {
    let descriptions = dxgi_adapters()?;
    let dxcore = dxcore_integrated();
    Ok(descriptions
        .iter()
        .filter(|d| is_hardware(d.Flags))
        .map(|d| {
            let luid = luid_to_u64(d.AdapterLuid);
            let (pci, adapter_type) = kernel_info(luid);
            Adapter {
                luid,
                name: adapter_name(&d.Description),
                vendor_id: d.VendorId,
                device_id: d.DeviceId,
                subsys_id: d.SubSysId,
                pci,
                integrated: integrated_from(dxcore.get(&luid).copied(), adapter_type),
                dedicated_bytes: d.DedicatedVideoMemory as u64,
            }
        })
        .collect())
}

/// LUID of the first integrated GPU, in DXGI order.
pub(crate) fn first_integrated(adapters: &[Adapter]) -> Option<u64> {
    adapters.iter().find(|a| a.integrated).map(|a| a.luid)
}

fn dxgi_adapters() -> Result<Vec<DXGI_ADAPTER_DESC1>, ProviderError> {
    let failed =
        |call: &str, e: windows::core::Error| ProviderError::Failed(format!("{call}: {e}"));
    // SAFETY: no preconditions; the factory is released when dropped.
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| failed("CreateDXGIFactory1", e))?;
    let mut descriptions = Vec::new();
    for index in 0u32.. {
        // SAFETY: `factory` is a live COM object; out-of-range indices return
        // DXGI_ERROR_NOT_FOUND.
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(failed("IDXGIFactory1::EnumAdapters1", e)),
        };
        // SAFETY: `adapter` is a live COM object.
        let description =
            unsafe { adapter.GetDesc1() }.map_err(|e| failed("IDXGIAdapter1::GetDesc1", e))?;
        descriptions.push(description);
    }
    Ok(descriptions)
}

/// LUID → DXCore `IsIntegrated`, for the adapters whose driver reports it.
/// Empty when DXCore is unavailable; callers then fall back to D3DKMT.
fn dxcore_integrated() -> HashMap<u64, bool> {
    dxcore_query().unwrap_or_else(|e| {
        tracing::debug!(error = %e, "DXCore unavailable; integrated GPUs are detected from the adapter type");
        HashMap::new()
    })
}

type CreateFactory = unsafe extern "system" fn(
    *const windows::core::GUID,
    *mut *mut std::ffi::c_void,
) -> windows::core::HRESULT;

fn optional_dxcore_factory(
    create: Option<CreateFactory>,
) -> windows::core::Result<IDXCoreAdapterFactory> {
    use windows::core::Interface;
    let create = create.ok_or_else(|| {
        windows::core::Error::from_hresult(windows::Win32::Foundation::E_NOINTERFACE)
    })?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: exact export ABI; the output receives the requested COM interface.
    unsafe { create(&IDXCoreAdapterFactory::IID, &mut raw) }.ok()?;
    if raw.is_null() {
        return Err(windows::core::Error::from_hresult(
            windows::Win32::Foundation::E_POINTER,
        ));
    }
    // SAFETY: successful factory creation transfers one owned COM reference.
    Ok(unsafe { IDXCoreAdapterFactory::from_raw(raw) })
}

fn dxcore_query() -> windows::core::Result<HashMap<u64, bool>> {
    static CREATE: std::sync::OnceLock<Option<CreateFactory>> = std::sync::OnceLock::new();
    let create = *CREATE.get_or_init(|| {
        let library = crate::dynlib::Library::system32("dxcore.dll").ok()?;
        // SAFETY: exact export ABI. Library deliberately keeps the module loaded.
        unsafe { library.symbol::<CreateFactory>(c"DXCoreCreateAdapterFactory") }
    });
    let factory = optional_dxcore_factory(create)?;
    // SAFETY: the attribute slice outlives the call.
    let list: IDXCoreAdapterList =
        unsafe { factory.CreateAdapterList(&[DXCORE_ADAPTER_ATTRIBUTE_D3D12_CORE_COMPUTE]) }?;
    let mut integrated = HashMap::new();
    // SAFETY: `list` is a live COM object.
    for index in 0..unsafe { list.GetAdapterCount() } {
        // SAFETY: `index` is below GetAdapterCount.
        let adapter: IDXCoreAdapter = unsafe { list.GetAdapter(index) }?;
        let mut luid = LUID::default();
        // SAFETY: InstanceLuid is an 8-byte LUID and the buffer is exactly that size.
        unsafe {
            adapter.GetProperty(
                InstanceLuid,
                size_of::<LUID>(),
                (&mut luid as *mut LUID).cast(),
            )
        }?;
        // SAFETY: `adapter` is a live COM object.
        if !unsafe { adapter.IsPropertySupported(IsIntegrated) } {
            continue;
        }
        let mut value = 0u8;
        // SAFETY: IsIntegrated is a 1-byte boolean and the buffer is 1 byte.
        if unsafe { adapter.GetProperty(IsIntegrated, 1, (&mut value as *mut u8).cast()) }.is_ok() {
            integrated.insert(luid_to_u64(luid), value != 0);
        }
    }
    Ok(integrated)
}

/// PCI address and ADAPTERTYPE bits from a short-lived D3DKMT handle.
fn kernel_info(luid: u64) -> (Option<PciAddress>, Option<u32>) {
    match KmtAdapter::open(luid) {
        Ok(kmt) => (
            kmt.address()
                .ok()
                .and_then(|a| pci_from_address(a.BusNumber, a.DeviceNumber, a.FunctionNumber)),
            kmt.adapter_type().ok(),
        ),
        Err(status) => {
            tracing::debug!(
                luid = format!("{luid:#x}"),
                status = format!("{:#010x}", status.0 as u32),
                "D3DKMTOpenAdapterFromLuid failed"
            );
            (None, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(luid: u64, integrated: bool) -> Adapter {
        Adapter {
            luid,
            name: String::new(),
            vendor_id: 0,
            device_id: 0,
            subsys_id: 0,
            pci: None,
            integrated,
            dedicated_bytes: 0,
        }
    }

    #[test]
    fn first_integrated_skips_discrete_adapters() {
        let list = [adapter(1, false), adapter(2, true), adapter(3, true)];
        assert_eq!(first_integrated(&list), Some(2));
        assert_eq!(first_integrated(&[adapter(1, false)]), None);
        assert_eq!(first_integrated(&[]), None);
    }

    #[test]
    fn luid_packs_high_and_low_parts() {
        let rtx = LUID {
            LowPart: 0x0001_7DB6,
            HighPart: 0,
        };
        assert_eq!(luid_to_u64(rtx), 0x17DB6);
        let high = LUID {
            LowPart: 0xFFFF_FFFF,
            HighPart: 1,
        };
        assert_eq!(luid_to_u64(high), 0x1_FFFF_FFFF);
        let negative = LUID {
            LowPart: 2,
            HighPart: -1,
        };
        assert_eq!(luid_to_u64(negative), 0xFFFF_FFFF_0000_0002);
    }

    #[test]
    fn luid_round_trips() {
        for value in [0x17DB6u64, 0x1_FFFF_FFFF, 0xFFFF_FFFF_0000_0002] {
            assert_eq!(luid_to_u64(u64_to_luid(value)), value);
        }
    }

    #[test]
    fn software_adapter_is_skipped() {
        // Flags observed on this machine: RTX 4080 and Radeon 0x0, Basic Render 0x2.
        assert!(is_hardware(0x0));
        assert!(!is_hardware(0x2));
        // DXGI_ADAPTER_FLAG_REMOTE (0x1) alone is still hardware.
        assert!(is_hardware(0x1));
        assert!(!is_hardware(0x3));
    }

    #[test]
    fn pci_address_from_kernel_values() {
        let rtx = pci_from_address(1, 0, 0).expect("pci");
        assert_eq!(rtx.to_string(), "0000:01:00.0");
        let radeon = pci_from_address(17, 0, 0).expect("pci");
        assert_eq!(radeon.to_string(), "0000:11:00.0");
    }

    #[test]
    fn adapter_without_bus_has_no_pci_address() {
        // Microsoft Basic Render Driver: bus 0xFFFFFFFF, device/function 0xFFFF.
        assert_eq!(pci_from_address(u32::MAX, 0xFFFF, 0xFFFF), None);
        assert_eq!(pci_from_address(1, 32, 0), None);
        assert_eq!(pci_from_address(1, 0, 8), None);
    }

    #[test]
    fn dxcore_decides_integrated_when_available() {
        // ADAPTERTYPE values observed: RTX 4080 0x231b, Radeon iGPU 0x2323.
        assert!(integrated_from(Some(true), Some(0x231B)));
        assert!(!integrated_from(Some(false), Some(0x2323)));
    }

    #[test]
    fn hybrid_integrated_bit_is_the_fallback() {
        assert!(integrated_from(None, Some(0x2323)));
        assert!(!integrated_from(None, Some(0x231B)));
        assert!(!integrated_from(None, None));
    }

    #[test]
    fn adapter_name_stops_at_nul_and_trims() {
        let mut buffer = [0u16; 128];
        for (slot, c) in buffer
            .iter_mut()
            .zip("NVIDIA GeForce RTX 4080  ".encode_utf16())
        {
            *slot = c;
        }
        assert_eq!(adapter_name(&buffer), "NVIDIA GeForce RTX 4080");
        let full: Vec<u16> = "AMD Radeon(TM) Graphics".encode_utf16().collect();
        assert_eq!(adapter_name(&full), "AMD Radeon(TM) Graphics");
    }

    #[test]
    fn missing_dxcore_export_uses_the_kernel_fallback() {
        assert!(optional_dxcore_factory(None).is_err());
        assert!(integrated_from(None, Some(HYBRID_INTEGRATED)));
        assert!(!integrated_from(None, None));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn enumerates_this_machines_gpus() {
        let adapters = enumerate().expect("enumerate");
        let summary: Vec<(String, bool)> = adapters
            .iter()
            .map(|a| {
                (
                    a.pci.map(|p| p.to_string()).unwrap_or_default(),
                    a.integrated,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("0000:01:00.0".to_owned(), false),
                ("0000:11:00.0".to_owned(), true)
            ],
            "{adapters:#?}"
        );
        let rtx = &adapters[0];
        assert_eq!(rtx.vendor_id, 0x10DE);
        assert!(rtx.name.contains("RTX 4080"), "{}", rtx.name);
        assert!(rtx.dedicated_bytes > 15 << 30, "{}", rtx.dedicated_bytes);
        let radeon = &adapters[1];
        assert_eq!(radeon.vendor_id, 0x1002);
        assert!(radeon.name.contains("Radeon"), "{}", radeon.name);
        assert_ne!(rtx.luid, radeon.luid);
    }
}

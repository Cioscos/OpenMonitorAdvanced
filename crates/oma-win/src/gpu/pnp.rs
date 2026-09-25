//! Vendor-neutral PCIe link capability from the Plug and Play property store (cfgmgr32).
//!
//! Each display adapter interface is opened once with D3DKMT to learn its LUID, then its
//! device node gives the PCI "max link speed/width" properties. These come from the device's
//! own Link Capabilities register, i.e. the device capability alone, independent of whatever
//! slot it is plugged into: unlike NVML's max-link getters (which report the maximum
//! "possible with this device AND system", so a x16 card in a x4 slot reports 4 there), the
//! PnP values do not change between normal and safe mode and agree across vendors. This is
//! why `pcieMaxGen`/`pcieMaxWidth` are sourced from PnP only (fix round 1) and not from NVML.
//! The PnP "current link" pair is a snapshot taken when the device started and is never
//! refreshed (it reads Gen 4 while NVML reports Gen 1 at idle), so it is not used.
//! The layer declares no sensors: it only contributes device properties.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem::size_of;

use oma_core::model::Source;
use oma_core::provider::ProviderError;
use windows::core::{GUID, PCWSTR};
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromDeviceName, D3DKMT_CLOSEADAPTER,
    D3DKMT_OPENADAPTERFROMDEVICENAME,
};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_PropertyW, CM_Locate_DevNodeW, CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
    CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_InstanceId, DEVPROPTYPE};
use windows::Win32::Foundation::DEVPROPKEY;

use super::adapter::Adapter;
use super::enumerate::luid_to_u64;
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};

const _: () = assert!(size_of::<D3DKMT_OPENADAPTERFROMDEVICENAME>() == 24);
const _: () = assert!(size_of::<DEVPROPKEY>() == 20);

/// Interface class of every display adapter (GUID_DISPLAY_DEVICE_ARRIVAL).
const DISPLAY_ADAPTER_INTERFACE: GUID = GUID::from_u128(0x1ca05180_a699_450a_9a0c_de4fbe3ddd89);
/// Property set of PCI devices; every key below is a DEVPROP_TYPE_UINT32.
const PCI_PROPERTY_SET: GUID = GUID::from_u128(0x3ab22e31_8264_4b4e_9af5_a8d2d8e33e62);
/// Highest link speed the device supports, as a generation number (4 = 16 GT/s).
const MAX_LINK_SPEED: DEVPROPKEY = DEVPROPKEY {
    fmtid: PCI_PROPERTY_SET,
    pid: 11,
};
/// Highest link width the device supports, in lanes.
const MAX_LINK_WIDTH: DEVPROPKEY = DEVPROPKEY {
    fmtid: PCI_PROPERTY_SET,
    pid: 12,
};
/// A device instance id has at most 200 characters (MAX_DEVICE_ID_LEN) plus the NUL.
const INSTANCE_ID_LEN: usize = 256;

/// `pcieMaxGen` / `pcieMaxWidth` from the raw PnP values; implausible values are left out.
pub(crate) fn link_properties(
    max_speed: Option<u32>,
    max_width: Option<u32>,
) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    if let Some(generation) = max_speed.filter(|g| (1..=7).contains(g)) {
        properties.insert("pcieMaxGen".to_owned(), generation.to_string());
    }
    if let Some(lanes) = max_width.filter(|w| (1..=32).contains(w)) {
        properties.insert("pcieMaxWidth".to_owned(), lanes.to_string());
    }
    properties
}

/// Splits a REG_MULTI_SZ-style list into its strings, each keeping its NUL terminator.
pub(crate) fn split_multi_sz(list: &[u16]) -> Vec<Vec<u16>> {
    list.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut z = s.to_vec();
            z.push(0);
            z
        })
        .collect()
}

/// Paths of the present display adapter interfaces (NUL-terminated UTF-16).
fn display_interfaces() -> Vec<Vec<u16>> {
    let mut len = 0u32;
    // SAFETY: `len` is a valid out pointer and the class GUID outlives the call.
    let cr = unsafe {
        CM_Get_Device_Interface_List_SizeW(
            &mut len,
            &DISPLAY_ADAPTER_INTERFACE,
            PCWSTR::null(),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };
    if cr != CR_SUCCESS || len == 0 {
        return Vec::new();
    }
    let mut list = vec![0u16; len as usize];
    // SAFETY: `list` has the length the size call returned.
    let cr = unsafe {
        CM_Get_Device_Interface_ListW(
            &DISPLAY_ADAPTER_INTERFACE,
            PCWSTR::null(),
            &mut list,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };
    if cr != CR_SUCCESS {
        return Vec::new();
    }
    split_multi_sz(&list)
}

/// LUID of the adapter behind an interface path, from a handle closed right away.
fn interface_luid(path: &[u16]) -> Option<u64> {
    let mut open = D3DKMT_OPENADAPTERFROMDEVICENAME {
        pDeviceName: PCWSTR(path.as_ptr()),
        ..Default::default()
    };
    // SAFETY: `path` is NUL-terminated and outlives the call; `open` is writable.
    if unsafe { D3DKMTOpenAdapterFromDeviceName(&mut open) }.0 < 0 {
        return None;
    }
    let close = D3DKMT_CLOSEADAPTER {
        hAdapter: open.hAdapter,
    };
    // SAFETY: the handle was just opened and is not used afterwards.
    unsafe {
        let _ = D3DKMTCloseAdapter(&close);
    }
    Some(luid_to_u64(open.AdapterLuid))
}

/// Device node of the device that exposes an interface path.
fn interface_devnode(path: &[u16]) -> Option<u32> {
    let mut kind = DEVPROPTYPE(0);
    let mut id = [0u16; INSTANCE_ID_LEN];
    let mut size = size_of::<[u16; INSTANCE_ID_LEN]>() as u32;
    // SAFETY: `path` is NUL-terminated; `id` is a writable buffer of `size` bytes.
    let cr = unsafe {
        CM_Get_Device_Interface_PropertyW(
            PCWSTR(path.as_ptr()),
            &DEVPKEY_Device_InstanceId,
            &mut kind,
            Some(id.as_mut_ptr().cast()),
            &mut size,
            0,
        )
    };
    if cr != CR_SUCCESS {
        return None;
    }
    let mut devnode = 0u32;
    // SAFETY: `id` holds a NUL-terminated instance id (the buffer was zeroed and is larger
    // than the longest id); `devnode` is a valid out pointer.
    let cr =
        unsafe { CM_Locate_DevNodeW(&mut devnode, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) };
    (cr == CR_SUCCESS).then_some(devnode)
}

/// LUID -> device node of every present display adapter.
fn devnodes_by_luid() -> HashMap<u64, u32> {
    display_interfaces()
        .iter()
        .filter_map(|path| Some((interface_luid(path)?, interface_devnode(path)?)))
        .collect()
}

/// A 32-bit device property; `None` when absent (e.g. a non-PCI adapter).
fn property_u32(devnode: u32, key: &DEVPROPKEY) -> Option<u32> {
    let mut kind = DEVPROPTYPE(0);
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is a writable buffer of `size` bytes and `key` outlives the call.
    let cr = unsafe {
        CM_Get_DevNode_PropertyW(
            devnode,
            key,
            &mut kind,
            Some((&mut value as *mut u32).cast()),
            &mut size,
            0,
        )
    };
    (cr == CR_SUCCESS && size == 4).then_some(value)
}

/// Base layer that only contributes the PCIe maximum link as device properties.
#[derive(Default)]
pub(crate) struct PnpLayer {
    /// Per adapter of the last attach.
    properties: Vec<BTreeMap<String, String>>,
}

impl GpuLayer for PnpLayer {
    fn source(&self) -> Source {
        Source::Pnp
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        let devnodes = devnodes_by_luid();
        self.properties = adapters
            .iter()
            .map(|adapter| match devnodes.get(&adapter.luid) {
                Some(&devnode) => link_properties(
                    property_u32(devnode, &MAX_LINK_SPEED),
                    property_u32(devnode, &MAX_LINK_WIDTH),
                ),
                None => BTreeMap::new(),
            })
            .collect();
        vec![BTreeSet::new(); adapters.len()]
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        Ok(vec![Readings::new(); self.properties.len()])
    }

    fn properties(&self, adapter: usize) -> BTreeMap<String, String> {
        self.properties.get(adapter).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_properties_keep_plausible_values_only() {
        assert_eq!(
            link_properties(Some(4), Some(16)),
            BTreeMap::from([
                ("pcieMaxGen".to_owned(), "4".to_owned()),
                ("pcieMaxWidth".to_owned(), "16".to_owned()),
            ])
        );
        assert_eq!(
            link_properties(Some(0), Some(64)),
            BTreeMap::new(),
            "zero generation and 64 lanes are not plausible"
        );
        assert_eq!(
            link_properties(None, Some(8)),
            BTreeMap::from([("pcieMaxWidth".to_owned(), "8".to_owned())])
        );
        assert_eq!(link_properties(None, None), BTreeMap::new());
    }

    #[test]
    fn multi_sz_list_is_split_into_terminated_strings() {
        let list: Vec<u16> = "ab\0cd\0\0".encode_utf16().collect();
        assert_eq!(
            split_multi_sz(&list),
            vec![
                "ab\0".encode_utf16().collect::<Vec<u16>>(),
                "cd\0".encode_utf16().collect::<Vec<u16>>(),
            ]
        );
        assert!(split_multi_sz(&[0, 0]).is_empty());
    }

    #[test]
    fn layer_without_attach_has_no_properties() {
        let layer = PnpLayer::default();
        assert_eq!(layer.source(), Source::Pnp);
        assert!(layer.properties(0).is_empty());
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn reads_the_max_link_of_both_gpus() {
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        let mut layer = PnpLayer::default();
        let fields = layer.attach(&adapters);
        assert_eq!(fields.len(), adapters.len());
        assert!(
            fields.iter().all(BTreeSet::is_empty),
            "PnP declares no sensors"
        );
        for vendor_id in [0x10DE, 0x1002] {
            let index = adapters
                .iter()
                .position(|a| a.vendor_id == vendor_id)
                .expect("adapter present");
            let properties = layer.properties(index);
            println!("{}: {properties:?}", adapters[index].name);
            assert_eq!(properties.get("pcieMaxGen").map(String::as_str), Some("4"));
            assert_eq!(
                properties.get("pcieMaxWidth").map(String::as_str),
                Some("16")
            );
        }
        assert_eq!(layer.sample().expect("sample").len(), adapters.len());
    }
}

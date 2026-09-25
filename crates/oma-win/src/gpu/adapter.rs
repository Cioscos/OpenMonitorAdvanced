//! A physical GPU as found by enumeration, and its stable device id.

use std::fmt;

/// PCI location of an adapter (segment 0 is assumed, as on every desktop board).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct PciAddress {
    pub bus: u32,
    pub device: u32,
    pub function: u32,
}

impl fmt::Display for PciAddress {
    /// "0000:01:00.0" (domain:bus:device.function), the form used by NVML and Device Manager.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "0000:{:02x}:{:02x}.{:x}",
            self.bus, self.device, self.function
        )
    }
}

/// GPU vendors that have a vendor layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Vendor {
    Nvidia,
    Amd,
    Intel,
}

impl Vendor {
    /// Maps a PCI vendor id; None for any other vendor (e.g. a virtual adapter).
    pub fn from_pci_id(id: u32) -> Option<Vendor> {
        match id {
            0x10DE => Some(Vendor::Nvidia),
            0x1002 => Some(Vendor::Amd),
            0x8086 => Some(Vendor::Intel),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Vendor::Nvidia => "NVIDIA",
            Vendor::Amd => "AMD",
            Vendor::Intel => "Intel",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Adapter {
    /// ((HighPart as u32 as u64) << 32) | LowPart as u64
    pub luid: u64,
    /// DXGI description, trimmed.
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub subsys_id: u32,
    pub pci: Option<PciAddress>,
    pub integrated: bool,
    /// DXGI DedicatedVideoMemory.
    pub dedicated_bytes: u64,
}

impl Adapter {
    pub fn vendor(&self) -> Option<Vendor> {
        Vendor::from_pci_id(self.vendor_id)
    }

    /// Stable device id: "gpu/pci-0000:01:00.0"; without a PCI address
    /// "gpu/ven-10de-dev-2704-{ordinal}", where `ordinal` counts only the
    /// adapters without a PCI address, in enumeration order.
    pub fn device_id(&self, ordinal: usize) -> String {
        match self.pci {
            Some(pci) => format!("gpu/pci-{pci}"),
            None => format!(
                "gpu/ven-{:04x}-dev-{:04x}-{ordinal}",
                self.vendor_id, self.device_id
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(vendor_id: u32, device_id: u32, pci: Option<PciAddress>) -> Adapter {
        Adapter {
            luid: 0x0000_0000_0001_2345,
            name: "GPU".into(),
            vendor_id,
            device_id,
            subsys_id: 0,
            pci,
            integrated: false,
            dedicated_bytes: 0,
        }
    }

    #[test]
    fn pci_address_uses_the_domain_bus_device_function_form() {
        let nvidia = PciAddress {
            bus: 1,
            device: 0,
            function: 0,
        };
        let amd = PciAddress {
            bus: 0x11,
            device: 0,
            function: 0,
        };
        let other = PciAddress {
            bus: 0xAB,
            device: 0x1F,
            function: 7,
        };
        assert_eq!(nvidia.to_string(), "0000:01:00.0");
        assert_eq!(amd.to_string(), "0000:11:00.0");
        assert_eq!(other.to_string(), "0000:ab:1f.7");
    }

    #[test]
    fn vendor_from_pci_id() {
        assert_eq!(Vendor::from_pci_id(0x10DE), Some(Vendor::Nvidia));
        assert_eq!(Vendor::from_pci_id(0x1002), Some(Vendor::Amd));
        assert_eq!(Vendor::from_pci_id(0x8086), Some(Vendor::Intel));
        assert_eq!(Vendor::from_pci_id(0x1414), None);
        assert_eq!(
            [Vendor::Nvidia, Vendor::Amd, Vendor::Intel].map(Vendor::name),
            ["NVIDIA", "AMD", "Intel"]
        );
    }

    #[test]
    fn device_id_prefers_the_pci_address() {
        let pci = Some(PciAddress {
            bus: 1,
            device: 0,
            function: 0,
        });
        assert_eq!(
            adapter(0x10DE, 0x2704, pci).device_id(3),
            "gpu/pci-0000:01:00.0"
        );
        assert_eq!(
            adapter(0x10DE, 0x2704, None).device_id(0),
            "gpu/ven-10de-dev-2704-0"
        );
        assert_eq!(
            adapter(0x8086, 0x56A0, None).device_id(1),
            "gpu/ven-8086-dev-56a0-1"
        );
        assert_eq!(adapter(0x1002, 0x164E, None).vendor(), Some(Vendor::Amd));
    }
}

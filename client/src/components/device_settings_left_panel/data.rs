//! Temporary device data for pre-join settings UI.
//!
//! NOTE: This file intentionally contains stubs. Replace these lists with
//! browser-provided devices from `navigator.mediaDevices.enumerateDevices()`
//! once media wiring is implemented.

/// Logical category of media device shown in the left-panel tabs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Input,
    Output,
    Camera,
}

impl DeviceKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Input => "Microphone",
            Self::Output => "Speakers",
            Self::Camera => "Camera",
        }
    }

    pub fn nav_label(self) -> &'static str {
        match self {
            Self::Input => "INPUT",
            Self::Output => "OUTPUT",
            Self::Camera => "CAMERA",
        }
    }
}

/// UI row model for a discovered media device.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DeviceEntry {
    /// Stable identifier for list selection.
    pub id: &'static str,
    /// Human readable label shown to the user.
    pub label: &'static str,
}

/// All device groups used by [`crate::components::DeviceSettingsLeftPanel`].
pub struct DeviceCatalog {
    pub inputs: &'static [DeviceEntry],
    pub outputs: &'static [DeviceEntry],
    pub cameras: &'static [DeviceEntry],
}

/// Returns static placeholder devices until runtime detection is connected.
///
/// TODO(real devices):
/// - request permissions on user action (mic/camera preview),
/// - call `enumerateDevices()`,
/// - group by kind (`audioinput`, `audiooutput`, `videoinput`),
/// - map to `DeviceEntry { id, label }`,
/// - fallback label when browser returns empty name before permission.
pub fn stub_device_catalog() -> DeviceCatalog {
    const INPUTS: &[DeviceEntry] = &[
        DeviceEntry {
            id: "mic-default",
            label: "USB Microphone (Default)",
        },
        DeviceEntry {
            id: "mic-realtek",
            label: "Realtek HD Audio Input",
        },
        DeviceEntry {
            id: "mic-virtual",
            label: "Virtual Cable",
        },
    ];

    const OUTPUTS: &[DeviceEntry] = &[
        DeviceEntry {
            id: "spk-default",
            label: "Headphones (Default)",
        },
        DeviceEntry {
            id: "spk-hd",
            label: "Speakers (High Definition)",
        },
        DeviceEntry {
            id: "spk-bt",
            label: "Bluetooth Hands-Free",
        },
    ];

    const CAMERAS: &[DeviceEntry] = &[
        DeviceEntry {
            id: "cam-integrated",
            label: "Integrated Webcam",
        },
        DeviceEntry {
            id: "cam-obs",
            label: "OBS Virtual Camera",
        },
    ];

    DeviceCatalog {
        inputs: INPUTS,
        outputs: OUTPUTS,
        cameras: CAMERAS,
    }
}


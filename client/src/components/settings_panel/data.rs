//! Temporary device data for settings panel UI.
//!
//! NOTE: This file intentionally contains stubs. Replace these lists with
//! browser-provided devices from `navigator.mediaDevices.enumerateDevices()`
//! once media wiring is implemented.

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DeviceEntry {
    pub id: &'static str,
    pub label: &'static str,
}

/// All device groups used by [`crate::components::SettingsPanel`].
pub struct DeviceCatalog {
    pub inputs: &'static [DeviceEntry],
    pub outputs: &'static [DeviceEntry],
    pub cameras: &'static [DeviceEntry],
}

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

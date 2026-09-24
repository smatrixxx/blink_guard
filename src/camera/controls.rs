use nokhwa::utils::{CameraFormat, ControlValueSetter, KnownCameraControl, Resolution};
use nokhwa::Camera;

pub fn select_optimal_format(cam: &mut Camera) -> Option<CameraFormat> {
    let formats = cam.compatible_camera_formats().ok()?;
    let target_res = Resolution::new(640, 480);

    for fmt in &formats {
        if fmt.resolution() == target_res && fmt.frame_rate() >= 30 {
            return Some(*fmt);
        }
    }

    for fmt in &formats {
        if fmt.resolution() == target_res {
            return Some(*fmt);
        }
    }

    formats.first().copied()
}

pub fn fix_camera_flicker(device_path: &str) {
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("v4l2-ctl")
            .args(["-d", device_path, "--set-ctrl=power_line_frequency=1"])
            .status();
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device_path;
    }
}

pub fn optimize_camera_for_tracking(cam: &mut Camera) {
    let _ = cam.set_camera_control(
        KnownCameraControl::WhiteBalance,
        ControlValueSetter::Integer(4000),
    );
}

pub fn set_brightness(cam: &mut Camera, value: i64) -> Result<(), String> {
    cam.set_camera_control(
        KnownCameraControl::Brightness,
        ControlValueSetter::Integer(value),
    )
    .map_err(|e| format!("{e}"))
}

pub fn set_contrast(cam: &mut Camera, value: i64) -> Result<(), String> {
    cam.set_camera_control(
        KnownCameraControl::Contrast,
        ControlValueSetter::Integer(value),
    )
    .map_err(|e| format!("{e}"))
}

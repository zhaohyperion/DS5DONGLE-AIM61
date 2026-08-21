//! Native layout adapter for `daidr/dualsense-tester`'s DualSense model.
//!
//! The coordinate system and control locations below are derived from
//! `src/router/DualSense/views/_ModelPanel/{DSBody,DSCover,DSBack}.vue` at
//! commit d85bbaf2cf6ade22aae3983f22c99a176e50c827 (MIT).

pub const SOURCE_COMMIT: &str = "d85bbaf2cf6ade22aae3983f22c99a176e50c827";
pub const VIEW_WIDTH: f32 = 1117.0;
pub const VIEW_HEIGHT: f32 = 892.0;

pub const DPAD_UP: (f32, f32) = (178.738, 295.9);
pub const DPAD_RIGHT: (f32, f32) = (239.9, 357.009);
pub const DPAD_DOWN: (f32, f32) = (178.738, 419.0);
pub const DPAD_LEFT: (f32, f32) = (117.6, 357.009);

pub const SQUARE: (f32, f32) = (864.079, 358.08);
pub const TRIANGLE: (f32, f32) = (934.079, 288.08);
pub const CIRCLE: (f32, f32) = (1004.08, 358.08);
pub const CROSS: (f32, f32) = (934.079, 428.08);

pub const LEFT_STICK: (f32, f32) = (351.764, 528.548);
pub const RIGHT_STICK: (f32, f32) = (763.456, 528.548);
pub const STICK_RANGE_RADIUS: f32 = 86.0;
pub const STICK_CAP_RADIUS: f32 = 57.193;

pub const TOUCHPAD_LEFT: f32 = 340.0;
pub const TOUCHPAD_TOP: f32 = 160.0;
pub const TOUCHPAD_WIDTH: f32 = 430.0;
pub const TOUCHPAD_HEIGHT: f32 = 235.0;
pub const TOUCHPAD_RANGE_X: f32 = 1920.0;
pub const TOUCHPAD_RANGE_Y: f32 = 1080.0;

pub fn normalize_stick(value: u8) -> f32 {
    (2.0 * value as f32) / 255.0 - 1.0
}

pub fn stick_offset(x: u8, y: u8) -> (f32, f32) {
    (
        normalize_stick(x) * STICK_RANGE_RADIUS,
        normalize_stick(y) * STICK_RANGE_RADIUS,
    )
}

pub fn touch_position(x: u16, y: u16) -> (f32, f32) {
    (
        TOUCHPAD_LEFT + x.min(1919) as f32 * TOUCHPAD_WIDTH / TOUCHPAD_RANGE_X,
        TOUCHPAD_TOP + y.min(1079) as f32 * TOUCHPAD_HEIGHT / TOUCHPAD_RANGE_Y,
    )
}

pub fn dpad_active(direction: u8) -> [bool; 4] {
    [
        matches!(direction, 0 | 1 | 7),
        matches!(direction, 1 | 2 | 3),
        matches!(direction, 3 | 4 | 5),
        matches!(direction, 5 | 6 | 7),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stick_normalization_matches_dualsense_tester() {
        assert_eq!(normalize_stick(0), -1.0);
        assert_eq!(normalize_stick(255), 1.0);
        assert!((normalize_stick(128) - (1.0 / 255.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn touch_mapping_uses_upstream_model_rectangle() {
        assert_eq!(touch_position(0, 0), (TOUCHPAD_LEFT, TOUCHPAD_TOP));
        let (x, y) = touch_position(1919, 1079);
        assert!(x < TOUCHPAD_LEFT + TOUCHPAD_WIDTH);
        assert!(y < TOUCHPAD_TOP + TOUCHPAD_HEIGHT);
    }

    #[test]
    fn diagonal_dpad_lights_both_physical_directions() {
        assert_eq!(dpad_active(1), [true, true, false, false]);
        assert_eq!(dpad_active(5), [false, false, true, true]);
        assert_eq!(dpad_active(8), [false; 4]);
    }
}

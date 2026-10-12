//! Read-only DualSense motion-sensor factory calibration.
//!
//! The feature-report layout and defensive denominator handling are
//! cross-checked against Bluepad32's Apache-2.0 DS5 parser and Linux
//! `hid-playstation`.  Only calibration constants are retained; Bluetooth
//! addresses, serial numbers and HID paths are deliberately excluded.

use serde::Serialize;

pub const CALIBRATION_REPORT_ID: u8 = 0x05;
const CALIBRATION_REPORT_BYTES: usize = 41;
const GYRO_RES_PER_DEGREE_SECOND: f32 = 1024.0;
const ACCEL_RES_PER_G: f32 = 8192.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CalibrationSource {
    ControllerFactory,
    M61CompatibilityFallback,
    NominalFallback,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionCalibrationInfo {
    pub source: CalibrationSource,
    pub factory_data_valid: bool,
    pub report_bytes: usize,
    pub gyro_axis_valid: [bool; 3],
    pub accelerometer_axis_valid: [bool; 3],
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct MotionCalibration {
    pub info: MotionCalibrationInfo,
    gyro_scale_dps: [f32; 3],
    accel_bias: [f32; 3],
    accel_scale_g: [f32; 3],
}

impl MotionCalibration {
    pub fn nominal(reason: impl Into<String>) -> Self {
        Self {
            info: MotionCalibrationInfo {
                source: CalibrationSource::NominalFallback,
                factory_data_valid: false,
                report_bytes: 0,
                gyro_axis_valid: [false; 3],
                accelerometer_axis_valid: [false; 3],
                warnings: vec![reason.into()],
            },
            gyro_scale_dps: [1.0 / GYRO_RES_PER_DEGREE_SECOND; 3],
            accel_bias: [0.0; 3],
            accel_scale_g: [1.0 / ACCEL_RES_PER_G; 3],
        }
    }

    pub fn calibrated_gyro_dps(&self, raw: [i16; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| raw[axis] as f32 * self.gyro_scale_dps[axis])
    }

    pub fn calibrated_accel_g(&self, raw: [i16; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| {
            (raw[axis] as f32 - self.accel_bias[axis]) * self.accel_scale_g[axis]
        })
    }
}

pub fn decode_calibration_report(report: &[u8]) -> Result<MotionCalibration, String> {
    if report.len() < CALIBRATION_REPORT_BYTES {
        return Err(format!(
            "calibration report is truncated: got {} bytes, expected at least {CALIBRATION_REPORT_BYTES}",
            report.len()
        ));
    }
    if report[0] != CALIBRATION_REPORT_ID {
        return Err(format!(
            "unexpected calibration report id 0x{:02x}",
            report[0]
        ));
    }

    let values = std::array::from_fn::<_, 17, _>(|index| {
        read_i16_le(report, 1 + index * 2).unwrap_or_default()
    });
    if values.iter().all(|value| *value == 0) || values.iter().all(|value| *value == -1) {
        return Err("calibration report contains only an empty transport pattern".to_owned());
    }

    let compatibility_fallback = values
        == [
            0, 0, 0, 8192, -8192, 8192, -8192, 8192, -8192, 1024, -1024, 8192, -8192, 8192, -8192,
            8192, -8192,
        ];
    let mut warnings = Vec::new();
    if compatibility_fallback {
        warnings.push(
            "M61 returned its compatibility calibration because live controller factory data was unavailable"
                .to_owned(),
        );
    }

    let speed_2x = i32::from(values[9]) + i32::from(values[10]);
    let gyro_bias = [values[0], values[1], values[2]];
    let gyro_plus = [values[3], values[5], values[7]];
    let gyro_minus = [values[4], values[6], values[8]];
    let mut gyro_axis_valid = [true; 3];
    let mut gyro_scale_dps = [1.0 / GYRO_RES_PER_DEGREE_SECOND; 3];
    for axis in 0..3 {
        let denominator = (i32::from(gyro_plus[axis]) - i32::from(gyro_bias[axis])).abs()
            + (i32::from(gyro_minus[axis]) - i32::from(gyro_bias[axis])).abs();
        if speed_2x == 0 || denominator == 0 {
            gyro_axis_valid[axis] = false;
            warnings.push(format!(
                "gyroscope axis {} calibration is invalid; nominal scaling is used",
                axis_name(axis)
            ));
        } else {
            // Linux first normalizes into 1/1024 degree/s. Dividing that
            // result by 1024 leaves this direct degree/s scale.
            gyro_scale_dps[axis] = speed_2x as f32 / denominator as f32;
        }
    }

    let accel_plus = [values[11], values[13], values[15]];
    let accel_minus = [values[12], values[14], values[16]];
    let mut accelerometer_axis_valid = [true; 3];
    let mut accel_bias = [0.0; 3];
    let mut accel_scale_g = [1.0 / ACCEL_RES_PER_G; 3];
    for axis in 0..3 {
        let range_2g = i32::from(accel_plus[axis]) - i32::from(accel_minus[axis]);
        if range_2g == 0 {
            accelerometer_axis_valid[axis] = false;
            warnings.push(format!(
                "accelerometer axis {} calibration is invalid; nominal scaling is used",
                axis_name(axis)
            ));
        } else {
            accel_bias[axis] = (i32::from(accel_plus[axis]) - range_2g / 2) as f32;
            accel_scale_g[axis] = 2.0 / range_2g as f32;
        }
    }

    let all_axes_valid = gyro_axis_valid.iter().all(|valid| *valid)
        && accelerometer_axis_valid.iter().all(|valid| *valid);
    Ok(MotionCalibration {
        info: MotionCalibrationInfo {
            source: if compatibility_fallback {
                CalibrationSource::M61CompatibilityFallback
            } else {
                CalibrationSource::ControllerFactory
            },
            factory_data_valid: !compatibility_fallback && all_axes_valid,
            report_bytes: report.len(),
            gyro_axis_valid,
            accelerometer_axis_valid,
            warnings,
        },
        gyro_scale_dps,
        accel_bias,
        accel_scale_g,
    })
}

#[cfg(windows)]
pub fn read_controller_motion_calibration(device: &hidapi::HidDevice) -> MotionCalibration {
    let mut last_error = None;
    for attempt in 0..3 {
        let mut report = [0_u8; 64];
        report[0] = CALIBRATION_REPORT_ID;
        match device.get_feature_report(&mut report) {
            Ok(length) => match decode_calibration_report(&report[..length]) {
                Ok(calibration) => return calibration,
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(format!("unable to read feature report 0x05: {error}")),
        }
        if attempt < 2 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    MotionCalibration::nominal(
        last_error.unwrap_or_else(|| "controller calibration is unavailable".to_owned()),
    )
}

fn read_i16_le(source: &[u8], offset: usize) -> Option<i16> {
    source
        .get(offset..offset + 2)?
        .try_into()
        .ok()
        .map(i16::from_le_bytes)
}

fn axis_name(axis: usize) -> char {
    ['X', 'Y', 'Z'][axis.min(2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(values: [i16; 17]) -> [u8; CALIBRATION_REPORT_BYTES] {
        let mut report = [0_u8; CALIBRATION_REPORT_BYTES];
        report[0] = CALIBRATION_REPORT_ID;
        for (index, value) in values.iter().enumerate() {
            report[1 + index * 2..3 + index * 2].copy_from_slice(&value.to_le_bytes());
        }
        report
    }

    #[test]
    fn decodes_factory_calibration_into_standard_units() {
        let calibration = decode_calibration_report(&report([
            12, -8, 4, 8100, -8200, 8120, -8180, 8090, -8210, 1100, 1100, 8200, -8100, 8150, -8250,
            8300, -8050,
        ]))
        .unwrap();
        assert_eq!(
            calibration.info.source,
            CalibrationSource::ControllerFactory
        );
        assert!(calibration.info.factory_data_valid);
        let gyro = calibration.calibrated_gyro_dps([1024, -1024, 512]);
        assert!(gyro[0] > 130.0 && gyro[0] < 140.0);
        let accel = calibration.calibrated_accel_g([50, -50, 125]);
        assert!(accel.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn identifies_m61_compatibility_values_as_non_factory_data() {
        let calibration = decode_calibration_report(&report([
            0, 0, 0, 8192, -8192, 8192, -8192, 8192, -8192, 1024, -1024, 8192, -8192, 8192, -8192,
            8192, -8192,
        ]))
        .unwrap();
        assert_eq!(
            calibration.info.source,
            CalibrationSource::M61CompatibilityFallback
        );
        assert!(!calibration.info.factory_data_valid);
        assert_eq!(calibration.calibrated_gyro_dps([1024, 0, 0])[0], 1.0);
        assert_eq!(calibration.calibrated_accel_g([8192, 0, 0])[0], 1.0);
    }

    #[test]
    fn invalid_axis_uses_safe_nominal_scaling() {
        let calibration = decode_calibration_report(&report([
            0, 0, 0, 0, 0, 8000, -8000, 8000, -8000, 1000, 1000, 0, 0, 8000, -8000, 8000, -8000,
        ]))
        .unwrap();
        assert_eq!(calibration.info.gyro_axis_valid, [false, true, true]);
        assert_eq!(
            calibration.info.accelerometer_axis_valid,
            [false, true, true]
        );
        assert!(calibration.calibrated_gyro_dps([1024, 0, 0])[0].is_finite());
        assert!(calibration.calibrated_accel_g([8192, 0, 0])[0].is_finite());
    }

    #[test]
    fn rejects_truncated_wrong_id_and_empty_reports() {
        assert!(decode_calibration_report(&[0x05; 12]).is_err());
        let mut wrong = report([1; 17]);
        wrong[0] = 0x20;
        assert!(decode_calibration_report(&wrong).is_err());
        assert!(decode_calibration_report(&report([0; 17])).is_err());
        assert!(decode_calibration_report(&report([-1; 17])).is_err());
    }
}

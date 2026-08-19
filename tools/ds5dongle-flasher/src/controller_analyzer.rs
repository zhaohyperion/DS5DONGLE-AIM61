use serde::Serialize;

use super::device_test::InputState;

pub const CIRCULARITY_BINS: usize = 48;
const CENTER_SAMPLE_TARGET: u16 = 120;
const REQUIRED_DIRECTION_COVERAGE_PERCENT: f32 = 75.0;
const CENTER_RETEST_PERCENT: f32 = 3.0;
const CENTER_CALIBRATE_PERCENT: f32 = 5.0;
const CIRCULARITY_TOO_LOW_PERCENT: f32 = 5.0;
const CIRCULARITY_RETEST_PERCENT: f32 = 12.0;
const CIRCULARITY_CALIBRATE_PERCENT: f32 = 20.0;
const RANGE_RETEST_PERCENT: f32 = 95.0;
const RANGE_CALIBRATE_PERCENT: f32 = 85.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QualityGrade {
    #[default]
    Incomplete,
    Normal,
    Retest,
    Calibrate,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickAssessment {
    pub grade: QualityGrade,
    pub center_offset_percent: Option<f32>,
    pub circularity_error_percent: Option<f32>,
    pub range_x_percent: f32,
    pub range_y_percent: f32,
}

#[derive(Clone, Debug)]
pub struct StickAnalysis {
    bins: [f32; CIRCULARITY_BINS],
    samples: u64,
    min_x: u8,
    max_x: u8,
    min_y: u8,
    max_y: u8,
}

impl Default for StickAnalysis {
    fn default() -> Self {
        Self {
            bins: [0.0; CIRCULARITY_BINS],
            samples: 0,
            min_x: u8::MAX,
            max_x: u8::MIN,
            min_y: u8::MAX,
            max_y: u8::MIN,
        }
    }
}

impl StickAnalysis {
    fn observe(&mut self, raw_x: u8, raw_y: u8) {
        let (x, y) = normalized_stick(raw_x, raw_y);
        let distance = x.hypot(y);
        if distance > 0.02 {
            let angle = y.atan2(x);
            let scaled =
                (angle * CIRCULARITY_BINS as f32 / (2.0 * std::f32::consts::PI)).round() as i32;
            let index = scaled.rem_euclid(CIRCULARITY_BINS as i32) as usize;
            self.bins[index] = self.bins[index].max(distance);
        }
        self.samples += 1;
        self.min_x = self.min_x.min(raw_x);
        self.max_x = self.max_x.max(raw_x);
        self.min_y = self.min_y.min(raw_y);
        self.max_y = self.max_y.max(raw_y);
    }

    pub fn bins(&self) -> &[f32; CIRCULARITY_BINS] {
        &self.bins
    }

    pub fn samples(&self) -> u64 {
        self.samples
    }

    pub fn coverage_percent(&self) -> f32 {
        self.bins.iter().filter(|value| **value > 0.3).count() as f32 * 100.0
            / CIRCULARITY_BINS as f32
    }

    /// RMS distance from the ideal unit circle. This deliberately matches
    /// dualshock-tools' public `calculateCircularityError` interoperability
    /// algorithm, including its 0.2 validity threshold.
    pub fn circularity_error_percent(&self) -> Option<f32> {
        let valid = self.bins.iter().filter(|value| **value > 0.2);
        let count = valid.clone().count();
        if count == 0 {
            return None;
        }
        let sum = valid.map(|value| (value - 1.0).powi(2)).sum::<f32>();
        Some((sum / count as f32).sqrt() * 100.0)
    }

    pub fn axis_range_percent(&self) -> (f32, f32) {
        if self.samples == 0 {
            return (0.0, 0.0);
        }
        (
            (self.max_x.saturating_sub(self.min_x)) as f32 / 255.0 * 100.0,
            (self.max_y.saturating_sub(self.min_y)) as f32 / 255.0 * 100.0,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickCenter {
    pub x: f32,
    pub y: f32,
    pub offset_percent: f32,
}

#[derive(Clone, Debug)]
pub struct ControllerAnalyzer {
    pub active: bool,
    pub left: StickAnalysis,
    pub right: StickAnalysis,
    pub left_center: Option<StickCenter>,
    pub right_center: Option<StickCenter>,
    center_samples_remaining: u16,
    center_samples_collected: u16,
    center_sums: [f32; 4],
    last_report_count: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StickAnalysisReport {
    pub samples: u64,
    pub direction_bins: Vec<f32>,
    pub direction_coverage_percent: f32,
    pub circularity_error_percent: Option<f32>,
    pub range_x_percent: f32,
    pub range_y_percent: f32,
    pub center: Option<StickCenter>,
    pub assessment: StickAssessment,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControllerAnalysisReport {
    pub complete: bool,
    pub recording: bool,
    pub required_direction_coverage_percent: f32,
    pub methodology: &'static str,
    pub thresholds_are_official: bool,
    pub left: StickAnalysisReport,
    pub right: StickAnalysisReport,
}

impl Default for ControllerAnalyzer {
    fn default() -> Self {
        Self {
            active: false,
            left: StickAnalysis::default(),
            right: StickAnalysis::default(),
            left_center: None,
            right_center: None,
            center_samples_remaining: 0,
            center_samples_collected: 0,
            center_sums: [0.0; 4],
            last_report_count: 0,
        }
    }
}

impl ControllerAnalyzer {
    pub fn reset(&mut self) {
        let active = self.active;
        *self = Self::default();
        self.active = active;
    }

    pub fn begin_center_capture(&mut self) {
        self.center_samples_remaining = CENTER_SAMPLE_TARGET;
        self.center_samples_collected = 0;
        self.center_sums = [0.0; 4];
        self.left_center = None;
        self.right_center = None;
    }

    pub fn center_capture_progress(&self) -> Option<f32> {
        if self.center_samples_remaining == 0 {
            return None;
        }
        Some(self.center_samples_collected as f32 / CENTER_SAMPLE_TARGET as f32)
    }

    pub fn read_only_analysis_complete(&self) -> bool {
        self.left_center.is_some()
            && self.right_center.is_some()
            && self.left.coverage_percent() >= REQUIRED_DIRECTION_COVERAGE_PERCENT
            && self.right.coverage_percent() >= REQUIRED_DIRECTION_COVERAGE_PERCENT
    }

    pub fn required_coverage_percent(&self) -> f32 {
        REQUIRED_DIRECTION_COVERAGE_PERCENT
    }

    pub fn left_assessment(&self) -> StickAssessment {
        assess_stick(&self.left, self.left_center)
    }

    pub fn right_assessment(&self) -> StickAssessment {
        assess_stick(&self.right, self.right_center)
    }

    pub fn report(&self) -> ControllerAnalysisReport {
        ControllerAnalysisReport {
            complete: self.read_only_analysis_complete(),
            recording: self.active,
            required_direction_coverage_percent: REQUIRED_DIRECTION_COVERAGE_PERCENT,
            methodology: "dualshock-tools-compatible 48-direction maximum-radius RMS circularity",
            thresholds_are_official: false,
            left: stick_report(&self.left, self.left_center),
            right: stick_report(&self.right, self.right_center),
        }
    }

    pub fn observe(&mut self, input: &InputState) {
        if input.report_count == 0 || input.report_count == self.last_report_count {
            return;
        }
        self.last_report_count = input.report_count;

        if self.active {
            self.left.observe(input.lx, input.ly);
            self.right.observe(input.rx, input.ry);
        }

        if self.center_samples_remaining > 0 {
            let (lx, ly) = normalized_stick(input.lx, input.ly);
            let (rx, ry) = normalized_stick(input.rx, input.ry);
            self.center_sums[0] += lx;
            self.center_sums[1] += ly;
            self.center_sums[2] += rx;
            self.center_sums[3] += ry;
            self.center_samples_collected += 1;
            self.center_samples_remaining -= 1;
            if self.center_samples_remaining == 0 {
                let divisor = self.center_samples_collected.max(1) as f32;
                let left = (self.center_sums[0] / divisor, self.center_sums[1] / divisor);
                let right = (self.center_sums[2] / divisor, self.center_sums[3] / divisor);
                self.left_center = Some(center_result(left));
                self.right_center = Some(center_result(right));
            }
        }
    }
}

pub fn normalized_stick(raw_x: u8, raw_y: u8) -> (f32, f32) {
    (
        (raw_x as f32 - 127.5) / 127.5,
        (raw_y as f32 - 127.5) / 127.5,
    )
}

fn center_result((x, y): (f32, f32)) -> StickCenter {
    StickCenter {
        x,
        y,
        offset_percent: x.hypot(y) * 100.0,
    }
}

fn assess_stick(stick: &StickAnalysis, center: Option<StickCenter>) -> StickAssessment {
    let circularity = stick.circularity_error_percent();
    let (range_x, range_y) = stick.axis_range_percent();
    let center_offset = center.map(|value| value.offset_percent);
    if center_offset.is_none()
        || circularity.is_none()
        || stick.coverage_percent() < REQUIRED_DIRECTION_COVERAGE_PERCENT
    {
        return StickAssessment {
            grade: QualityGrade::Incomplete,
            center_offset_percent: center_offset,
            circularity_error_percent: circularity,
            range_x_percent: range_x,
            range_y_percent: range_y,
        };
    }

    let center_offset = center_offset.unwrap_or_default();
    let circularity = circularity.unwrap_or_default();
    let minimum_range = range_x.min(range_y);
    let grade = if center_offset > CENTER_CALIBRATE_PERCENT
        || circularity > CIRCULARITY_CALIBRATE_PERCENT
        || minimum_range < RANGE_CALIBRATE_PERCENT
    {
        QualityGrade::Calibrate
    } else if center_offset > CENTER_RETEST_PERCENT
        || circularity < CIRCULARITY_TOO_LOW_PERCENT
        || circularity > CIRCULARITY_RETEST_PERCENT
        || minimum_range < RANGE_RETEST_PERCENT
    {
        QualityGrade::Retest
    } else {
        QualityGrade::Normal
    };
    StickAssessment {
        grade,
        center_offset_percent: Some(center_offset),
        circularity_error_percent: Some(circularity),
        range_x_percent: range_x,
        range_y_percent: range_y,
    }
}

fn stick_report(stick: &StickAnalysis, center: Option<StickCenter>) -> StickAnalysisReport {
    let (range_x, range_y) = stick.axis_range_percent();
    StickAnalysisReport {
        samples: stick.samples(),
        direction_bins: stick.bins().to_vec(),
        direction_coverage_percent: stick.coverage_percent(),
        circularity_error_percent: stick.circularity_error_percent(),
        range_x_percent: range_x,
        range_y_percent: range_y,
        center,
        assessment: assess_stick(stick, center),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_uses_symmetric_half_step_center() {
        let (low, _) = normalized_stick(127, 128);
        let (high, _) = normalized_stick(128, 128);
        assert!((low + high).abs() < f32::EPSILON);
        assert!((normalized_stick(0, 0).0 + 1.0).abs() < f32::EPSILON);
        assert!((normalized_stick(255, 255).0 - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn circularity_matches_reference_rms_definition() {
        let mut analysis = StickAnalysis::default();
        analysis.bins.fill(1.0);
        assert_eq!(analysis.circularity_error_percent(), Some(0.0));
        analysis.bins.fill(0.9);
        assert!((analysis.circularity_error_percent().unwrap() - 10.0).abs() < 0.001);
    }

    #[test]
    fn empty_analysis_has_no_error_result() {
        assert!(
            StickAnalysis::default()
                .circularity_error_percent()
                .is_none()
        );
    }

    #[test]
    fn permanent_calibration_requires_center_and_both_stick_coverage() {
        let mut analyzer = ControllerAnalyzer::default();
        analyzer.left_center = Some(StickCenter::default());
        analyzer.right_center = Some(StickCenter::default());
        analyzer.left.bins.fill(1.0);
        assert!(!analyzer.read_only_analysis_complete());
        analyzer.right.bins.fill(1.0);
        assert!(analyzer.read_only_analysis_complete());
    }

    #[test]
    fn assessment_uses_documented_three_level_thresholds() {
        let mut stick = StickAnalysis::default();
        stick.samples = 200;
        stick.min_x = 0;
        stick.max_x = 255;
        stick.min_y = 0;
        stick.max_y = 255;
        stick.bins.fill(0.92);
        assert_eq!(
            assess_stick(&stick, Some(StickCenter::default())).grade,
            QualityGrade::Normal
        );
        stick.bins.fill(1.0);
        assert_eq!(
            assess_stick(&stick, Some(StickCenter::default())).grade,
            QualityGrade::Retest
        );
        stick.bins.fill(0.92);
        assert_eq!(
            assess_stick(
                &stick,
                Some(StickCenter {
                    offset_percent: 4.0,
                    ..StickCenter::default()
                })
            )
            .grade,
            QualityGrade::Retest
        );
        stick.max_x = 180;
        assert_eq!(
            assess_stick(&stick, Some(StickCenter::default())).grade,
            QualityGrade::Calibrate
        );
    }
}

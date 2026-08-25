use serde::Serialize;
use std::collections::BTreeMap;

use super::device_test::InputState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PhaseResult {
    Pending,
    Pass,
    NotEffective,
    Skipped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultSource {
    #[default]
    None,
    Automatic,
    User,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseRecord {
    pub id: &'static str,
    pub title_zh: &'static str,
    pub title_en: &'static str,
    pub instruction_zh: &'static str,
    pub instruction_en: &'static str,
    pub target: &'static str,
    pub automatic: bool,
    pub requirements: BTreeMap<String, u32>,
    pub samples: BTreeMap<String, u32>,
    pub result: PhaseResult,
    pub result_source: ResultSource,
    pub note: String,
}

impl PhaseRecord {
    pub fn completed_requirements(&self) -> usize {
        self.requirements
            .iter()
            .filter(|(key, target)| self.samples.get(*key).copied().unwrap_or_default() >= **target)
            .count()
    }

    pub fn coverage_percent(&self) -> f32 {
        if self.requirements.is_empty() {
            return 0.0;
        }
        self.completed_requirements() as f32 / self.requirements.len() as f32
    }

    pub fn coverage_complete(&self) -> bool {
        !self.requirements.is_empty() && self.completed_requirements() == self.requirements.len()
    }

    pub fn missing_requirements(&self) -> Vec<(&str, u32, u32)> {
        self.requirements
            .iter()
            .filter_map(|(key, target)| {
                let observed = self.samples.get(key).copied().unwrap_or_default();
                (observed < *target).then_some((key.as_str(), observed, *target))
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug)]
struct TouchTrace {
    id: u8,
    start_x: u16,
    start_y: u16,
}

#[derive(Clone, Debug)]
pub struct GuidedTest {
    pub active: bool,
    pub current: usize,
    pub phases: Vec<PhaseRecord>,
    previous: InputState,
    synchronize_next_input: bool,
    touch_traces: [Option<TouchTrace>; 2],
}

impl Default for GuidedTest {
    fn default() -> Self {
        Self {
            active: false,
            current: 0,
            phases: phase_catalog(),
            previous: InputState::default(),
            synchronize_next_input: true,
            touch_traces: [None, None],
        }
    }
}

impl GuidedTest {
    pub fn start(&mut self) {
        *self = Self::default();
        self.active = true;
    }

    pub fn phase(&self) -> &PhaseRecord {
        &self.phases[self.current.min(self.phases.len() - 1)]
    }

    pub fn phase_mut(&mut self) -> &mut PhaseRecord {
        let current = self.current.min(self.phases.len() - 1);
        &mut self.phases[current]
    }

    pub fn mark_and_next(&mut self, result: PhaseResult) -> Vec<(&'static str, &'static str)> {
        self.phase_mut().result = result;
        self.phase_mut().result_source = ResultSource::User;
        if self.current + 1 < self.phases.len() {
            self.current += 1;
        } else {
            self.active = false;
        }
        self.auto_advance_ready_phases()
    }

    pub fn retest(&mut self) {
        let phase = self.phase_mut();
        phase.samples.clear();
        phase.result = PhaseResult::Pending;
        phase.result_source = ResultSource::None;
        phase.note.clear();
    }

    pub fn finish_summary(&mut self) -> bool {
        if !self.active || self.phase().id != "summary" {
            return false;
        }
        let phase = self.phase_mut();
        phase.result = PhaseResult::Pass;
        phase.result_source = ResultSource::Automatic;
        self.active = false;
        true
    }

    pub fn require_input_resync(&mut self) {
        self.synchronize_next_input = true;
        self.touch_traces = [None, None];
    }

    pub fn observe(&mut self, input: &InputState) -> Vec<(&'static str, &'static str)> {
        if self.synchronize_next_input {
            self.previous = input.clone();
            self.synchronize_next_input = false;
            return Vec::new();
        }
        if !self.active {
            self.previous = input.clone();
            return Vec::new();
        }

        for (name, now, before) in [
            ("square", input.square, self.previous.square),
            ("cross", input.cross, self.previous.cross),
            ("circle", input.circle, self.previous.circle),
            ("triangle", input.triangle, self.previous.triangle),
            ("create", input.create, self.previous.create),
            ("options", input.options, self.previous.options),
            ("ps", input.ps, self.previous.ps),
            ("mute", input.mute, self.previous.mute),
        ] {
            if now && !before {
                self.bump_phase("buttons", name);
            }
        }
        if input.dpad != self.previous.dpad && input.dpad < 8 {
            self.bump_phase("buttons", &format!("dpad{}", input.dpad));
        }

        for (name, now, before) in [
            ("l1", input.l1, self.previous.l1),
            ("r1", input.r1, self.previous.r1),
            ("l3", input.l3, self.previous.l3),
            ("r3", input.r3, self.previous.r3),
        ] {
            if now && !before {
                self.bump_phase("shoulders", name);
            }
        }

        self.observe_stick(
            "left",
            input.lx,
            input.ly,
            self.previous.lx,
            self.previous.ly,
        );
        self.observe_stick(
            "right",
            input.rx,
            input.ry,
            self.previous.rx,
            self.previous.ry,
        );
        self.observe_trigger(
            "l2",
            input.l2,
            self.previous.l2,
            input.l2_button,
            self.previous.l2_button,
        );
        self.observe_trigger(
            "r2",
            input.r2,
            self.previous.r2,
            input.r2_button,
            self.previous.r2_button,
        );
        self.observe_touch(input);
        self.observe_motion(input);

        self.previous = input.clone();
        self.auto_advance_ready_phases()
    }

    fn observe_stick(&mut self, prefix: &str, x: u8, y: u8, px: u8, py: u8) {
        if x.abs_diff(128) < 10
            && y.abs_diff(128) < 10
            && (px.abs_diff(128) >= 10 || py.abs_diff(128) >= 10)
        {
            self.bump_phase("sticks", &format!("{prefix}Center"));
        }
        let sector = stick_sector(x, y);
        if sector != stick_sector(px, py)
            && let Some(sector) = sector
        {
            self.bump_phase("sticks", &format!("{prefix}{}", STICK_DIRECTIONS[sector]));
        }
    }

    fn observe_trigger(&mut self, prefix: &str, value: u8, previous: u8, pressed: bool, was: bool) {
        if pressed && !was {
            self.bump_phase("triggers", &format!("{prefix}Button"));
        }
        if value >= 20 && previous < 20 {
            self.bump_phase("triggers", &format!("{prefix}Press"));
        }
        if crossed_band(previous, value, 96, 160) {
            self.bump_phase("triggers", &format!("{prefix}Mid"));
        }
        if value >= 235 && previous < 235 {
            self.bump_phase("triggers", &format!("{prefix}Full"));
        }
        if value < 20 && previous >= 20 {
            self.bump_phase("triggers", &format!("{prefix}Release"));
        }
    }

    fn observe_touch(&mut self, input: &InputState) {
        if input.touchpad_click && !self.previous.touchpad_click {
            self.bump_phase("touchpad", "click");
        }
        if input.touch[0].active
            && input.touch[1].active
            && !(self.previous.touch[0].active && self.previous.touch[1].active)
        {
            self.bump_phase("touchpad", "twoFinger");
        }
        for index in 0..2 {
            let now = input.touch[index];
            let before = self.previous.touch[index];
            if now.active && (!before.active || before.id != now.id) {
                self.touch_traces[index] = Some(TouchTrace {
                    id: now.id,
                    start_x: now.x,
                    start_y: now.y,
                });
                self.bump_phase("touchpad", if index == 0 { "finger1" } else { "finger2" });
                self.bump_phase(
                    "touchpad",
                    if now.x < 960 {
                        "leftRegion"
                    } else {
                        "rightRegion"
                    },
                );
            } else if !now.active
                && before.active
                && let Some(trace) = self.touch_traces[index].take()
                && trace.id == before.id
            {
                let dx = i32::from(before.x) - i32::from(trace.start_x);
                let dy = i32::from(before.y) - i32::from(trace.start_y);
                if dx.abs().max(dy.abs()) >= 220 {
                    let direction = if dx.abs() >= dy.abs() {
                        if dx > 0 { "swipeRight" } else { "swipeLeft" }
                    } else if dy > 0 {
                        "swipeDown"
                    } else {
                        "swipeUp"
                    };
                    self.bump_phase("touchpad", direction);
                }
            }
        }
    }

    fn observe_motion(&mut self, input: &InputState) {
        for (key, value, previous) in [
            ("gyroX", input.gyro_x, self.previous.gyro_x),
            ("gyroY", input.gyro_y, self.previous.gyro_y),
            ("gyroZ", input.gyro_z, self.previous.gyro_z),
        ] {
            if value.unsigned_abs() >= 500 && previous.unsigned_abs() < 500 {
                self.bump_phase("motion", key);
            }
        }
        for (key, value, previous) in [
            ("accelX", input.accel_x, self.previous.accel_x),
            ("accelY", input.accel_y, self.previous.accel_y),
            ("accelZ", input.accel_z, self.previous.accel_z),
        ] {
            if value.abs_diff(previous) >= 600 {
                self.bump_phase("motion", key);
            }
        }
    }

    fn bump_phase(&mut self, phase_id: &str, key: &str) {
        if let Some(phase) = self.phases.iter_mut().find(|phase| phase.id == phase_id) {
            *phase.samples.entry(key.to_owned()).or_default() += 1;
        }
    }

    fn auto_advance_ready_phases(&mut self) -> Vec<(&'static str, &'static str)> {
        let mut completed = Vec::new();
        while self.active && self.phase().automatic && self.phase().coverage_complete() {
            let phase = self.phase_mut();
            phase.result = PhaseResult::Pass;
            phase.result_source = ResultSource::Automatic;
            completed.push((phase.title_zh, phase.title_en));
            if self.current + 1 < self.phases.len() {
                self.current += 1;
            } else {
                self.active = false;
            }
        }
        completed
    }
}

const STICK_DIRECTIONS: [&str; 8] = [
    "Right",
    "UpRight",
    "Up",
    "UpLeft",
    "Left",
    "DownLeft",
    "Down",
    "DownRight",
];

fn stick_sector(x: u8, y: u8) -> Option<usize> {
    let dx = (x as f32 - 127.5) / 127.5;
    let dy = (127.5 - y as f32) / 127.5;
    if dx.hypot(dy) < 0.72 {
        return None;
    }
    let scaled = (dy.atan2(dx) * 4.0 / std::f32::consts::PI).round() as i32;
    Some(scaled.rem_euclid(8) as usize)
}

fn crossed_band(previous: u8, value: u8, low: u8, high: u8) -> bool {
    (value >= low && value <= high)
        || (previous < low && value > high)
        || (previous > high && value < low)
}

fn requirements(keys: &[&str]) -> BTreeMap<String, u32> {
    keys.iter().map(|key| ((*key).to_owned(), 1)).collect()
}

fn requirements_for_phase(id: &str) -> BTreeMap<String, u32> {
    match id {
        "buttons" => requirements(&[
            "square", "cross", "circle", "triangle", "create", "options", "ps", "mute", "dpad0",
            "dpad1", "dpad2", "dpad3", "dpad4", "dpad5", "dpad6", "dpad7",
        ]),
        "shoulders" => requirements(&["l1", "r1", "l3", "r3"]),
        "sticks" => requirements(&[
            "leftRight",
            "leftUpRight",
            "leftUp",
            "leftUpLeft",
            "leftLeft",
            "leftDownLeft",
            "leftDown",
            "leftDownRight",
            "leftCenter",
            "rightRight",
            "rightUpRight",
            "rightUp",
            "rightUpLeft",
            "rightLeft",
            "rightDownLeft",
            "rightDown",
            "rightDownRight",
            "rightCenter",
        ]),
        "triggers" => requirements(&[
            "l2Button",
            "l2Press",
            "l2Mid",
            "l2Full",
            "l2Release",
            "r2Button",
            "r2Press",
            "r2Mid",
            "r2Full",
            "r2Release",
        ]),
        "touchpad" => requirements(&[
            "finger1",
            "finger2",
            "leftRegion",
            "rightRegion",
            "swipeLeft",
            "swipeRight",
            "swipeUp",
            "swipeDown",
            "twoFinger",
            "click",
        ]),
        "motion" => requirements(&["gyroX", "gyroY", "gyroZ", "accelX", "accelY", "accelZ"]),
        _ => BTreeMap::new(),
    }
}

fn phase_catalog() -> Vec<PhaseRecord> {
    let rows = [
        (
            "precheck",
            "设备与音频端点预检",
            "Device/audio precheck",
            "确认目标是经 M61 连接的手柄，并检查扬声器、耳机和麦克风端点。",
            "Select the controller routed through M61 and verify speaker, headset and microphone endpoints.",
            "1 confirmation",
        ),
        (
            "buttons",
            "方向键与面键",
            "D-pad and buttons",
            "逐个按下所有面键、功能键和方向键八个方向；程序自动记录，全部覆盖后进入下一步。",
            "Press every face/function button and all eight D-pad directions; the test advances automatically after full coverage.",
            "all buttons + 8 D-pad directions",
        ),
        (
            "shoulders",
            "肩键与摇杆按压",
            "Shoulders and stick clicks",
            "分别按 L1、R1、L3、R3；全部识别后自动进入下一步。",
            "Press L1, R1, L3 and R3; the test advances automatically when all are detected.",
            "L1/R1/L3/R3",
        ),
        (
            "sticks",
            "摇杆范围与回中",
            "Stick range and center",
            "左右摇杆各沿外圈转动一周并至少松手回中一次；程序检查八方向与回中覆盖。",
            "Rotate each stick around the outer edge and release it to center; all eight directions and center return are detected automatically.",
            "8 directions + center per stick",
        ),
        (
            "triggers",
            "L2/R2 模拟量",
            "L2/R2 analog",
            "分别完整压下并松开 L2/R2；程序检查按键位、起始、中段、满量程和释放。",
            "Fully pull and release L2/R2; button, initial, middle, full-scale and release states are detected automatically.",
            "button + low/mid/full/release per trigger",
        ),
        (
            "touchpad",
            "触摸板",
            "Touchpad",
            "触摸左右区域，完成四向滑动和双指触摸，并按下触摸板；全部由程序自动识别。",
            "Touch the left/right regions, swipe in four directions, use two fingers and click the pad; all actions are detected automatically.",
            "left/right + 4 swipes + 2 fingers + click",
        ),
        (
            "motion",
            "陀螺仪与加速度计",
            "Gyro and accelerometer",
            "绕三个轴转动并平移手柄，程序自动检查陀螺仪和加速度计六轴是否都有响应。",
            "Rotate and translate the controller around all axes; all gyro and accelerometer axes are checked automatically.",
            "gyro XYZ + accelerometer XYZ",
        ),
        (
            "leds",
            "灯效",
            "LEDs",
            "工具生成灯条、玩家灯和静音灯信号；观察后确认。",
            "The tool generates lightbar, player and mute LED signals; observe and confirm.",
            "3 rounds",
        ),
        (
            "rumble",
            "左右震动",
            "Left/right rumble",
            "工具以不超过 35% 强度分别产生左右震动；触感确认。",
            "The tool generates left/right rumble at no more than 35%; confirm physically.",
            "3 rounds each",
        ),
        (
            "triggers_output",
            "左右自适应扳机",
            "Adaptive triggers",
            "工具对左右扳机施加受限阻力并自动复位，共重复 3 轮；确认左右效果。",
            "The tool applies limited resistance to both triggers and resets them for 3 rounds; confirm both sides.",
            "3 rounds each",
        ),
        (
            "sound",
            "声音",
            "Sound",
            "使用与 ds.evua.cc 一致的 0x80 参考序列播放手柄扬声器/耳机 1 kHz 信号。",
            "Play controller speaker/headset 1 kHz using the frozen ds.evua.cc-compatible 0x80 sequence.",
            "3 rounds each endpoint",
        ),
        (
            "microphone",
            "麦克风",
            "Microphone",
            "先验证静音灯，再通过 M61 USB 音频端点录音 20 秒、静音 5 秒并回放。",
            "Verify the mute light, then record 20 seconds plus 5 seconds silence through the M61 USB audio endpoint and play it back.",
            "20 seconds voice + 5 seconds silence",
        ),
        (
            "summary",
            "复位与总结",
            "Reset and summary",
            "停止所有输出、复位扳机并检查诊断快照后完成。",
            "Stop every output, reset triggers and review the diagnostic snapshot.",
            "all outputs reset",
        ),
    ];
    rows.into_iter()
        .map(|row| PhaseRecord {
            id: row.0,
            title_zh: row.1,
            title_en: row.2,
            instruction_zh: row.3,
            instruction_en: row.4,
            target: row.5,
            automatic: matches!(
                row.0,
                "buttons" | "shoulders" | "sticks" | "triggers" | "touchpad" | "motion"
            ),
            requirements: requirements_for_phase(row.0),
            samples: BTreeMap::new(),
            result: PhaseResult::Pending,
            result_source: ResultSource::None,
            note: String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_is_a_cleanup_step_and_finishes_automatically() {
        let mut guide = GuidedTest::default();
        guide.start();
        guide.current = guide.phases.len() - 1;
        assert!(guide.finish_summary());
        assert!(!guide.active);
        assert_eq!(guide.phase().result, PhaseResult::Pass);
        assert_eq!(guide.phase().result_source, ResultSource::Automatic);
        assert!(!guide.finish_summary());
    }

    #[test]
    fn repeated_button_presses_are_counted() {
        let mut guide = GuidedTest::default();
        guide.start();
        guide.current = 1;
        let mut input = InputState::default();
        guide.observe(&input);
        for _ in 0..10 {
            input.cross = true;
            guide.observe(&input);
            input.cross = false;
            guide.observe(&input);
        }
        assert_eq!(guide.phase().samples.get("cross"), Some(&10));
    }

    #[test]
    fn reconnect_resync_does_not_create_a_false_button_press() {
        let mut guide = GuidedTest::default();
        guide.start();
        guide.current = 1;
        let mut input = InputState {
            cross: true,
            ..InputState::default()
        };
        guide.observe(&input);
        assert!(guide.phase().samples.is_empty());

        input.cross = false;
        guide.observe(&input);
        input.cross = true;
        guide.observe(&input);
        assert_eq!(guide.phase().samples.get("cross"), Some(&1));

        guide.require_input_resync();
        guide.observe(&input);
        assert_eq!(guide.phase().samples.get("cross"), Some(&1));
    }

    #[test]
    fn not_effective_result_is_preserved_when_advancing() {
        let mut guide = GuidedTest::default();
        guide.start();
        guide.phase_mut().note = "no response".to_owned();
        let _ = guide.mark_and_next(PhaseResult::NotEffective);
        assert_eq!(guide.current, 1);
        assert_eq!(guide.phases[0].result, PhaseResult::NotEffective);
        assert_eq!(guide.phases[0].result_source, ResultSource::User);
        assert_eq!(guide.phases[0].note, "no response");
    }

    #[test]
    fn input_coverage_is_recorded_before_its_phase_and_auto_advances() {
        let mut guide = GuidedTest::default();
        guide.start();
        let mut input = InputState::default();
        guide.observe(&input);
        for key in [
            "square", "cross", "circle", "triangle", "create", "options", "ps", "mute",
        ] {
            match key {
                "square" => input.square = true,
                "cross" => input.cross = true,
                "circle" => input.circle = true,
                "triangle" => input.triangle = true,
                "create" => input.create = true,
                "options" => input.options = true,
                "ps" => input.ps = true,
                "mute" => input.mute = true,
                _ => unreachable!(),
            }
            guide.observe(&input);
            input.square = false;
            input.cross = false;
            input.circle = false;
            input.triangle = false;
            input.create = false;
            input.options = false;
            input.ps = false;
            input.mute = false;
            guide.observe(&input);
        }
        for dpad in 0..8 {
            input.dpad = dpad;
            guide.observe(&input);
            input.dpad = 8;
            guide.observe(&input);
        }
        assert!(guide.phases[1].coverage_complete());
        let auto = guide.mark_and_next(PhaseResult::Pass);
        assert_eq!(auto, vec![("方向键与面键", "D-pad and buttons")]);
        assert_eq!(guide.current, 2);
        assert_eq!(guide.phases[1].result, PhaseResult::Pass);
        assert_eq!(guide.phases[1].result_source, ResultSource::Automatic);
    }

    #[test]
    fn stick_sector_and_trigger_sweep_cover_objective_states() {
        assert_eq!(stick_sector(255, 128), Some(0));
        assert_eq!(stick_sector(255, 0), Some(1));
        assert_eq!(stick_sector(128, 0), Some(2));
        assert_eq!(stick_sector(128, 128), None);
        assert!(crossed_band(0, 255, 96, 160));
        assert!(crossed_band(255, 0, 96, 160));
    }

    #[test]
    fn touch_swipe_and_two_finger_are_detected() {
        let mut guide = GuidedTest::default();
        guide.start();
        let mut input = InputState::default();
        guide.observe(&input);
        input.touch[0] = super::super::device_test::TouchPoint {
            active: true,
            id: 1,
            x: 100,
            y: 500,
        };
        guide.observe(&input);
        input.touch[0].x = 500;
        guide.observe(&input);
        input.touch[1] = super::super::device_test::TouchPoint {
            active: true,
            id: 2,
            x: 1500,
            y: 500,
        };
        guide.observe(&input);
        input.touch[0].active = false;
        guide.observe(&input);
        let touch = guide
            .phases
            .iter()
            .find(|phase| phase.id == "touchpad")
            .unwrap();
        assert_eq!(touch.samples.get("swipeRight"), Some(&1));
        assert_eq!(touch.samples.get("twoFinger"), Some(&1));
        assert_eq!(touch.samples.get("leftRegion"), Some(&1));
        assert_eq!(touch.samples.get("rightRegion"), Some(&1));
    }
}

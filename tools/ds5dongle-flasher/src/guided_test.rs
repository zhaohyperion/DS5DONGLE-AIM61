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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseRecord {
    pub id: &'static str,
    pub title_zh: &'static str,
    pub title_en: &'static str,
    pub instruction_zh: &'static str,
    pub instruction_en: &'static str,
    pub target: &'static str,
    pub samples: BTreeMap<String, u32>,
    pub result: PhaseResult,
    pub note: String,
}

#[derive(Clone, Debug)]
pub struct GuidedTest {
    pub active: bool,
    pub current: usize,
    pub phases: Vec<PhaseRecord>,
    previous: InputState,
}

impl Default for GuidedTest {
    fn default() -> Self {
        Self {
            active: false,
            current: 0,
            phases: phase_catalog(),
            previous: InputState::default(),
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

    pub fn mark_and_next(&mut self, result: PhaseResult) {
        self.phase_mut().result = result;
        if self.current + 1 < self.phases.len() {
            self.current += 1;
        } else {
            self.active = false;
        }
    }

    pub fn retest(&mut self) {
        let phase = self.phase_mut();
        phase.samples.clear();
        phase.result = PhaseResult::Pending;
        phase.note.clear();
    }

    pub fn observe(&mut self, input: &InputState) {
        if !self.active {
            self.previous = input.clone();
            return;
        }
        let id = self.phase().id;
        match id {
            "buttons" => {
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
                        self.bump(name);
                    }
                }
                if input.dpad != self.previous.dpad && input.dpad < 8 {
                    self.bump(&format!("dpad{}", input.dpad));
                }
            }
            "shoulders" => {
                for (name, now, before) in [
                    ("l1", input.l1, self.previous.l1),
                    ("r1", input.r1, self.previous.r1),
                    ("l3", input.l3, self.previous.l3),
                    ("r3", input.r3, self.previous.r3),
                ] {
                    if now && !before {
                        self.bump(name);
                    }
                }
            }
            "sticks" => {
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
            }
            "triggers" => {
                if input.l2 > 220 && self.previous.l2 <= 220 {
                    self.bump("l2Full");
                }
                if input.r2 > 220 && self.previous.r2 <= 220 {
                    self.bump("r2Full");
                }
                if input.l2 < 20 && self.previous.l2 >= 20 {
                    self.bump("l2Release");
                }
                if input.r2 < 20 && self.previous.r2 >= 20 {
                    self.bump("r2Release");
                }
            }
            "touchpad" => {
                if input.touchpad_click && !self.previous.touchpad_click {
                    self.bump("click");
                }
                for index in 0..2 {
                    if input.touch[index].active && !self.previous.touch[index].active {
                        self.bump(if index == 0 { "finger1" } else { "finger2" });
                    }
                }
            }
            "motion" => {
                let motion = input.gyro_x.unsigned_abs() as u32
                    + input.gyro_y.unsigned_abs() as u32
                    + input.gyro_z.unsigned_abs() as u32;
                if motion > 900 {
                    self.bump("motionSamples");
                }
            }
            _ => {}
        }
        self.previous = input.clone();
    }

    fn observe_stick(&mut self, prefix: &str, x: u8, y: u8, px: u8, py: u8) {
        if x.abs_diff(128) < 10
            && y.abs_diff(128) < 10
            && (px.abs_diff(128) >= 10 || py.abs_diff(128) >= 10)
        {
            self.bump(&format!("{prefix}Center"));
        }
        for (name, reached, was) in [
            ("Left", x < 20, px < 20),
            ("Right", x > 235, px > 235),
            ("Up", y < 20, py < 20),
            ("Down", y > 235, py > 235),
        ] {
            if reached && !was {
                self.bump(&format!("{prefix}{name}"));
            }
        }
    }

    fn bump(&mut self, key: &str) {
        *self.phase_mut().samples.entry(key.to_owned()).or_default() += 1;
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
            "每个方向与数字按键可重复按压；采样充足后手动进入下一步。",
            "Press every direction and digital button repeatedly; continue when samples are sufficient.",
            "10 presses each + adjacent D-pad transitions",
        ),
        (
            "shoulders",
            "肩键与摇杆按压",
            "Shoulders and stick clicks",
            "分别重复按 L1、R1、L3、R3。",
            "Repeatedly press L1, R1, L3 and R3.",
            "10 presses each",
        ),
        (
            "sticks",
            "摇杆范围与回中",
            "Stick range and center",
            "左右摇杆各顺/逆时针旋转并多次松手回中。",
            "Rotate each stick clockwise/counter-clockwise and release to center repeatedly.",
            "5 rotations each direction + 20 centers",
        ),
        (
            "triggers",
            "L2/R2 模拟量",
            "L2/R2 analog",
            "慢压、快速压到底并保持，重复多次。",
            "Use slow pulls, fast full pulls and full holds repeatedly.",
            "10 slow + 10 fast + 3 full holds each",
        ),
        (
            "touchpad",
            "触摸板",
            "Touchpad",
            "左右区域点击、四向滑动、双指操作并按下触摸板。",
            "Test left/right touches, four swipe directions, two fingers and physical click.",
            "10 left/right + 5 swipes each + 5 two-finger + 10 clicks",
        ),
        (
            "motion",
            "陀螺仪与加速度计",
            "Gyro and accelerometer",
            "绕三轴转动手柄，然后静置 5 秒。",
            "Move the controller around all three axes, then keep it still for 5 seconds.",
            "5 movements per axis + 5 seconds still",
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
            "工具逐步施加受限阻力并自动复位；确认左右效果。",
            "The tool applies limited gradual resistance and resets it; confirm both sides.",
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
            samples: BTreeMap::new(),
            result: PhaseResult::Pending,
            note: String::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_button_presses_are_counted() {
        let mut guide = GuidedTest::default();
        guide.start();
        guide.current = 1;
        let mut input = InputState::default();
        for _ in 0..10 {
            input.cross = true;
            guide.observe(&input);
            input.cross = false;
            guide.observe(&input);
        }
        assert_eq!(guide.phase().samples.get("cross"), Some(&10));
    }
}

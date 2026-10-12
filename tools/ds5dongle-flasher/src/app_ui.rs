use super::*;

fn runtime_health_finding_text(
    language: Language,
    finding: &diagnostics::RuntimeHealthFinding,
) -> String {
    let observed = match finding.unit {
        "permille" => format!("{:.1}%", finding.observed as f64 / 10.0),
        "us" => format!("{} µs", finding.observed),
        "bytes" => format_bytes(finding.observed.min(u64::from(u32::MAX)) as u32),
        "words" => format!("{} words / {} B", finding.observed, finding.observed * 4),
        _ => format!("{} {}", finding.observed, finding.unit),
    };
    match finding.code {
        "task_stack_critical" | "task_stack_low" => format!(
            "{}: {} {} ({})",
            finding.component,
            language.tr("任务栈余量偏低", "task stack headroom is low"),
            observed,
            language.tr(
                "建议观察长测最低值",
                "observe the minimum during a long run"
            )
        ),
        "task_cpu_critical" | "task_cpu_high" => format!(
            "{}: {} {}",
            finding.component,
            language.tr("任务 CPU 占用偏高", "task CPU use is high"),
            observed
        ),
        "heap_headroom_critical" | "heap_headroom_low" => format!(
            "{}: {}",
            language.tr("最低堆余量偏低", "Minimum heap headroom is low"),
            observed
        ),
        "bridge_output_queue_saturated" => language
            .tr(
                "输出桥接队列在本次运行中到达过容量上限",
                "The output bridge queue reached capacity during this run",
            )
            .to_owned(),
        "bridge_p99_critical" | "bridge_p99_high" => format!(
            "{}: {}",
            language.tr("M61 内部 P99 延迟偏高", "M61 internal P99 latency is high"),
            observed
        ),
        "bridge_timing_saturated" => language
            .tr(
                "内部延迟计数发生饱和，至少一次延迟超过 65535 µs",
                "Internal timing saturated; at least one latency exceeded 65535 µs",
            )
            .to_owned(),
        _ => format!("{}: {}", finding.code, observed),
    }
}

impl eframe::App for FlasherApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        self.process_events();
        self.process_device_test_events();
        if ctx.input(|input| input.key_pressed(eframe::egui::Key::Escape)) {
            self.device_test_output = device_test::OutputState::default();
            self.device_test_controller_tone = None;
            if let Some(session) = &self.device_test_session {
                let _ = session.stop_all();
            }
            self.device_test_status = self
                .language
                .tr(
                    "紧急停止：全部输出和扳机已复位",
                    "Emergency stop: all outputs and triggers reset",
                )
                .to_owned();
        }
        let debug_now = Instant::now();
        let benchmark_due = self.device_debug_started.is_some_and(|started| {
            debug_now.duration_since(started)
                >= Duration::from_secs(self.device_debug_duration_secs as u64)
        });
        if benchmark_due {
            self.stop_debug_benchmark(true);
            self.status = if self.device_debug_complete {
                self.language
                    .tr(
                        "压力测试已达到设定时长，正在采集最终 M61 快照。",
                        "The stress test reached its configured duration; capturing the final M61 snapshot.",
                    )
                    .to_owned()
            } else {
                self.language
                    .tr(
                        "压力测试已结束，但未产生完整结果，请查看日志。",
                        "The stress test ended without a complete result; see the log for details.",
                    )
                    .to_owned()
            };
        }
        let benchmark_has_no_samples = self.device_debug_started.is_some_and(|started| {
            debug_now.duration_since(started) >= Duration::from_secs(3)
                && self.device_debug_metrics.sample_count == 0
        });
        if benchmark_has_no_samples {
            self.stop_debug_benchmark(false);
            self.status = self
                .language
                .tr(
                    "性能测试启动失败：3 秒内没有收到可计时的 HID 报告，已停止而不是继续生成空报告。请重新连接手柄后重试。",
                    "Performance test startup failed: no timed HID report arrived within 3 seconds. The test stopped instead of producing an empty report. Reconnect the controller and retry.",
                )
                .to_owned();
            self.append_log(self.status.clone());
        }
        let new_severe_gap = self.device_debug_metrics.maximum_interval_ms >= 20.0
            && (self.device_debug_alert_snapshot_max_ms < 20.0
                || self.device_debug_metrics.maximum_interval_ms
                    >= self.device_debug_alert_snapshot_max_ms + 5.0);
        let alert_cooldown_ready = self
            .device_debug_last_alert_snapshot
            .is_none_or(|last| debug_now.duration_since(last) >= Duration::from_secs(5));
        if self.device_debug_started.is_some()
            && new_severe_gap
            && alert_cooldown_ready
            && !self.loading_diagnostics
        {
            self.device_debug_alert_snapshot_max_ms = self.device_debug_metrics.maximum_interval_ms;
            self.device_debug_last_alert_snapshot = Some(debug_now);
            self.device_debug_next_snapshot = Some(debug_now + Duration::from_secs(5));
            self.capture_debug_snapshot();
        } else if self.device_debug_started.is_some()
            && self
                .device_debug_next_snapshot
                .is_some_and(|deadline| debug_now >= deadline)
            && !self.loading_diagnostics
        {
            self.device_debug_next_snapshot = Some(debug_now + Duration::from_secs(5));
            self.capture_debug_snapshot();
        }
        let busy = self.busy.is_some();
        let language = self.language;
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Title(format!(
            "{} {FLASHER_VERSION}",
            language.tr("DS5DONGLE-AIM61 刷写器", "DS5DONGLE-AIM61 Flasher")
        )));
        if busy && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::CancelClose);
            self.status = language
                .tr(
                    "当前操作尚未结束，暂时不能关闭窗口。",
                    "The current operation is still running; the window cannot close yet.",
                )
                .to_owned();
        }

        eframe::egui::TopBottomPanel::top("header")
            .frame(
                eframe::egui::Frame::new()
                    .fill(COLOR_HEADER)
                    .inner_margin(eframe::egui::Margin::symmetric(20, 14)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            eframe::egui::RichText::new("DS5DONGLE · AIM61")
                                .size(22.0)
                                .color(COLOR_TEXT_PRIMARY)
                                .strong(),
                        );
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "手柄功能测试、运行诊断与安全固件刷写",
                                "Controller tests, runtime diagnostics and safe firmware flashing",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                    });
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            eframe::egui::ComboBox::from_id_salt("language_combo")
                                .selected_text(self.language.display_name())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.language,
                                        Language::ZhCn,
                                        Language::ZhCn.display_name(),
                                    );
                                    ui.selectable_value(
                                        &mut self.language,
                                        Language::En,
                                        Language::En.display_name(),
                                    );
                                });
                            ui.label(
                                eframe::egui::RichText::new(format!("v{FLASHER_VERSION}"))
                                    .color(COLOR_ACCENT_HOVER)
                                    .monospace(),
                            );
                        },
                    );
                });
            });

        let previous_tab = self.current_tab;
        eframe::egui::TopBottomPanel::top("main_tabs")
            .frame(
                eframe::egui::Frame::new()
                    .fill(COLOR_HEADER)
                    .inner_margin(eframe::egui::Margin::symmetric(20, 8)),
            )
            .show(ctx, |ui| {
                let tab_width = ((ui.available_width() - 30.0) / 4.0).clamp(112.0, 190.0);
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (AppTab::TestCenter, language.tr("测试中心", "Test center")),
                        (
                            AppTab::ButtonMapping,
                            language.tr("按键映射", "Button mapping"),
                        ),
                        (AppTab::DeviceDebug, language.tr("设备调试", "Device debug")),
                        (AppTab::Flasher, language.tr("固件刷写", "Firmware flasher")),
                    ] {
                        let selected = self.current_tab == tab;
                        if ui.add(tab_button(label, selected, tab_width)).clicked() {
                            self.current_tab = tab;
                        }
                    }
                });
            });
        if previous_tab != self.current_tab && self.current_tab == AppTab::TestCenter {
            self.open_device_test();
        } else if previous_tab != self.current_tab && self.current_tab == AppTab::ButtonMapping {
            self.ensure_device_session();
            self.refresh_button_mapping();
            self.refresh_device_macros();
        } else if previous_tab != self.current_tab && self.current_tab == AppTab::DeviceDebug {
            self.ensure_device_session();
            self.refresh_polling_rate();
        }

        eframe::egui::TopBottomPanel::bottom("footer")
            .frame(
                eframe::egui::Frame::new()
                    .fill(COLOR_HEADER)
                    .inner_margin(eframe::egui::Margin::symmetric(20, 7)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(eframe::egui::RichText::new("●").color(COLOR_SUCCESS));
                    ui.label(
                        eframe::egui::RichText::new(language.tr(
                            "本机运行 · 诊断数据不会上传",
                            "Runs locally · diagnostic data is never uploaded",
                        ))
                        .color(COLOR_TEXT_MUTED)
                        .small(),
                    );
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            ui.label(
                                eframe::egui::RichText::new("zhaohyperion/DS5DONGLE-AIM61")
                                    .color(COLOR_TEXT_MUTED)
                                    .small()
                                    .monospace(),
                            );
                        },
                    );
                });
            });

        if self.language != language {
            self.relocalize_status();
            ctx.request_repaint();
        }

        if self.current_tab == AppTab::ButtonMapping {
            let mut refresh_mapping = false;
            let mut apply_mapping = false;
            let mut reset_mapping = false;
            let mut refresh_macros = false;
            let mut apply_macros = false;
            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame::new().fill(COLOR_APP_BG).inner_margin(20))
                .show(ctx, |ui| {
                    eframe::egui::ScrollArea::vertical()
                        .id_salt("mapping_page_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            page_heading(
                                ui,
                                language.tr("控制器配置", "Controller profile"),
                                language.tr("按键映射与宏", "Mapping and macros"),
                                language.tr(
                                    "使用 DualSense 图形编辑 19 控件映射、同步组合与设备端宏。所有修改先保存在草稿中，应用后自动读回校验。",
                                    "Edit all 19 controls, synchronous combinations, and device macros on the DualSense model. Changes remain drafts until applied and verified.",
                                ),
                            );
                            ui.add_space(14.0);
                            let sub_tab_width =
                                ((ui.available_width() - 20.0) / 3.0).clamp(140.0, 240.0);
                            ui.horizontal_wrapped(|ui| {
                                for (tab, zh, en) in [
                                    (MappingSubTab::Mapping, "19 键映射", "19-control mapping"),
                                    (MappingSubTab::MacroEditor, "宏编辑器", "Macro editor"),
                                    (MappingSubTab::DeviceMacros, "录制与设备宏", "Recording & device macros"),
                                ] {
                                    let selected = self.mapping_sub_tab == tab;
                                    if ui
                                        .add(tab_button(language.tr(zh, en), selected, sub_tab_width))
                                        .clicked()
                                    {
                                        self.mapping_sub_tab = tab;
                                    }
                                }
                            });
                            ui.add_space(12.0);

                            if self.mapping_sub_tab == MappingSubTab::Mapping {
                                surface_frame().show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.heading(language.tr("映射草稿", "Mapping draft"));
                                        ui.with_layout(
                                            eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                                            |ui| {
                                                if self.loading_button_mapping || self.applying_button_mapping {
                                                    ui.spinner();
                                                }
                                                ui.monospace(if self.selected_button_mapping.is_identity() {
                                                    language.tr("默认一对一", "Default one-to-one")
                                                } else {
                                                    language.tr("自定义/组合", "Custom / combo")
                                                });
                                            },
                                        );
                                    });
                                    ui.columns(2, |columns| {
                                        for (index, column) in columns.iter_mut().enumerate() {
                                            column.label(
                                                eframe::egui::RichText::new(if index == 0 {
                                                    language.tr("物理输入", "Physical input")
                                                } else {
                                                    language.tr("逻辑输出", "Logical output")
                                                })
                                                .color(COLOR_TEXT_PRIMARY)
                                                .strong(),
                                            );
                                            column.add(eframe::egui::Image::new(eframe::egui::include_image!(
                                                "../assets/dualsense-front.svg"
                                            ))
                                            .max_width(430.0));
                                        }
                                    });
                                });
                                ui.add_space(12.0);

                                let mapping_enabled = self.device_button_mapping.is_some()
                                    && !self.loading_button_mapping
                                    && !self.applying_button_mapping
                                    && self.busy.is_none();
                                surface_frame().show(ui, |ui| {
                                    ui.heading(language.tr("物理输入 → 逻辑输出", "Physical input → logical output"));
                                    ui.label(
                                        eframe::egui::RichText::new(language.tr(
                                            "未勾选任何输出会禁用该物理键；勾选多个输出会在同一个 HID 帧中同步按下。超过 4 个输出仅提示负载警告，不会阻止保存。",
                                            "No target disables a source. Multiple targets are pressed atomically in one HID frame. More than four targets shows a warning but remains allowed.",
                                        ))
                                        .color(COLOR_TEXT_MUTED),
                                    );
                                    ui.add_space(8.0);
                                    ui.add_enabled_ui(mapping_enabled, |ui| {
                                        for source in 0..device_config::REMAP_CONTROL_COUNT {
                                            let count = self.selected_button_mapping.target_count(source);
                                            let summary = if count == 0 {
                                                language.tr("已禁用", "Disabled").to_owned()
                                            } else {
                                                format!("{} {}", count, language.tr("个输出", "targets"))
                                            };
                                            eframe::egui::CollapsingHeader::new(format!(
                                                "{}  →  {}",
                                                remap_control_label(source, language),
                                                summary
                                            ))
                                            .id_salt(format!("mapping_source_{source}"))
                                            .show(ui, |ui| {
                                                ui.horizontal_wrapped(|ui| {
                                                    for target in 0..device_config::REMAP_CONTROL_COUNT {
                                                        let mut enabled = self
                                                            .selected_button_mapping
                                                            .is_target_enabled(source, target);
                                                        if ui
                                                            .checkbox(
                                                                &mut enabled,
                                                                remap_control_label(target, language),
                                                            )
                                                            .changed()
                                                        {
                                                            self.selected_button_mapping
                                                                .set_target_enabled(source, target, enabled);
                                                        }
                                                    }
                                                });
                                                if self.selected_button_mapping.target_count(source) > 4 {
                                                    ui.colored_label(
                                                        COLOR_WARNING,
                                                        language.tr(
                                                            "该输入会同时产生超过 4 个输出，请先进行最终输出验证。",
                                                            "This source emits more than four targets; validate the final output first.",
                                                        ),
                                                    );
                                                }
                                            });
                                        }
                                    });
                                    ui.add_space(10.0);
                                    ui.horizontal_wrapped(|ui| {
                                        let changed = self.device_button_mapping.as_ref().is_some_and(|current| {
                                            current.target_masks != self.selected_button_mapping.target_masks
                                        });
                                        if ui
                                            .add_enabled(mapping_enabled && changed, primary_button(language.tr(
                                                "应用到设备并校验",
                                                "Apply and verify",
                                            )))
                                            .clicked()
                                        {
                                            apply_mapping = true;
                                        }
                                        if ui
                                            .add_enabled(mapping_enabled, eframe::egui::Button::new(language.tr(
                                                "恢复默认映射",
                                                "Restore defaults",
                                            )))
                                            .clicked()
                                        {
                                            reset_mapping = true;
                                        }
                                        if ui
                                            .add_enabled(!self.loading_button_mapping && !self.applying_button_mapping,
                                                eframe::egui::Button::new(language.tr("重新读取", "Read again")))
                                            .clicked()
                                        {
                                            refresh_mapping = true;
                                        }
                                    });
                                    if self.selected_button_mapping.imported_legacy_v2 {
                                        ui.add_space(8.0);
                                        notice(ui, NoticeTone::Info,
                                            language.tr("已导入旧 19 键映射", "Legacy 19-control mapping imported"),
                                            language.tr(
                                                "旧版单目标映射已转换为 v3 草稿；升级到 v3.6.0 后才能写入组合键。",
                                                "The old single-target table was converted to a v3 draft. Upgrade to v3.6.0 before writing combinations.",
                                            ));
                                    }
                                    if let Some(error) = &self.button_mapping_error {
                                        ui.add_space(8.0);
                                        notice(ui, NoticeTone::Warning,
                                            language.tr("映射不可用", "Mapping unavailable"), error);
                                    }
                                });
                            } else if self.mapping_sub_tab == MappingSubTab::MacroEditor {
                                surface_frame().show(ui, |ui| {
                                    section_heading(
                                        ui,
                                        language.tr("宏时间轴编辑器", "Macro timeline editor"),
                                        language.tr(
                                            "建立触发规则，并用时间轴精确编辑按键、摇杆、扳机和触摸输入。",
                                            "Build trigger rules and precisely edit buttons, sticks, triggers, and touch input on a timeline.",
                                        ),
                                    );
                                    ui.add_space(10.0);
                                    macro_editor_ui(
                                        ui,
                                        &mut self.macro_set,
                                        &mut self.selected_macro_index,
                                        &mut self.macro_status,
                                        language,
                                    );
                                });
                            } else {
                                surface_frame().show(ui, |ui| {
                                    section_heading(
                                        ui,
                                        language.tr("独立录制与设备宏", "Standalone recording and device macros"),
                                        language.tr(
                                            "管理设备端运行开关、档位与 A/B 持久化存储。",
                                            "Manage runtime switches, profiles, and transactional A/B storage on the device.",
                                        ),
                                    );
                                    ui.add_space(10.0);
                                    raised_frame().show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.checkbox(
                                            &mut self.macro_set.globally_enabled,
                                            language.tr("时序宏总开关", "Master macro switch"),
                                        );
                                        ui.checkbox(
                                            &mut self.macro_set.sequence_enabled,
                                            language.tr("单次/按住时序宏", "One-shot / held sequences"),
                                        );
                                        ui.checkbox(
                                            &mut self.macro_set.repeat_enabled,
                                            language.tr("连发/重复/循环", "Turbo / repeat / loops"),
                                        );
                                        eframe::egui::ComboBox::from_id_salt("active_macro_profile")
                                            .selected_text(format!("{} {}", language.tr("档位", "Profile"), self.macro_set.active_profile + 1))
                                            .show_ui(ui, |ui| {
                                                for profile in 0..macro_config::PROFILE_COUNT {
                                                    ui.selectable_value(
                                                        &mut self.macro_set.active_profile,
                                                        profile,
                                                        format!("{} {}", language.tr("档位", "Profile"), profile + 1),
                                                    );
                                                }
                                            });
                                    });
                                    });
                                    ui.add_space(8.0);
                                    ui.horizontal_wrapped(|ui| {
                                        if self.loading_device_macros || self.applying_device_macros {
                                            ui.spinner();
                                        }
                                        if ui
                                            .add_enabled(
                                                !self.loading_device_macros
                                                    && !self.applying_device_macros,
                                                secondary_button(language.tr(
                                                    "从设备重新读取",
                                                    "Read from device",
                                                )),
                                            )
                                            .clicked()
                                        {
                                            refresh_macros = true;
                                        }
                                        if ui
                                            .add_enabled(
                                                !self.loading_device_macros
                                                    && !self.applying_device_macros
                                                    && self.macro_set.validate().is_ok(),
                                                primary_button(language.tr(
                                                    "写入设备并校验",
                                                    "Write and verify",
                                                )),
                                            )
                                            .clicked()
                                        {
                                            apply_macros = true;
                                        }
                                        if ui
                                            .add_enabled(
                                                !self.loading_device_macros
                                                    && !self.applying_device_macros
                                                    && self.busy.is_none(),
                                                danger_button(language.tr(
                                                    "清除设备宏数据…",
                                                    "Erase device macro data…",
                                                )),
                                            )
                                            .clicked()
                                        {
                                            self.confirm_macro_clear = true;
                                        }
                                        ui.monospace(format!(
                                            "{} {}",
                                            language.tr("设备代数", "Device generation"),
                                            self.macro_set.generation
                                        ));
                                    });
                                    if let Some(status) = &self.macro_status {
                                        ui.add_space(6.0);
                                        notice(
                                            ui,
                                            status_tone(status),
                                            language.tr("设备操作", "Device operation"),
                                            status,
                                        );
                                    }
                                    ui.add_space(8.0);
                                    notice(
                                        ui,
                                        NoticeTone::Info,
                                        language.tr("手柄端独立录制", "Standalone controller recording"),
                                        language.tr(
                                            "Create + Options 长按 2 秒进入准备；选择播放触发键；黄色倒计时 3 秒后绿灯常亮并开始录制；再次长按结束。达到 80% 容量时红灯慢闪，空间耗尽时红灯快闪。PS 长按 2 秒随时取消。",
                                            "Hold Create + Options for 2 seconds, choose the playback trigger, then recording starts with a solid green light after a 3-second amber countdown. Hold the chord again to finish. Red flashes at 80% capacity and rapidly when full. Hold PS for 2 seconds to cancel.",
                                        ),
                                    );
                                    ui.add_space(8.0);
                                    match self.macro_set.validate() {
                                        Ok(summary) => {
                                            ui.label(format!(
                                                "{} macros · {} steps · {} / {} bytes",
                                                summary.macro_count,
                                                summary.step_count,
                                                summary.compiled_bytes,
                                                macro_config::MAX_DEVICE_BYTES
                                            ));
                                            ui.add(
                                                eframe::egui::ProgressBar::new(
                                                    summary.compiled_bytes as f32 / macro_config::MAX_DEVICE_BYTES as f32,
                                                )
                                                .desired_width(ui.available_width().min(520.0))
                                                .text(format!("{:.1}%", summary.compiled_bytes as f32 * 100.0 / macro_config::MAX_DEVICE_BYTES as f32)),
                                            );
                                        }
                                        Err(error) => notice(
                                            ui,
                                            NoticeTone::Warning,
                                            language.tr("设备宏校验失败", "Device macro validation failed"),
                                            &format!("{error:#}"),
                                        ),
                                    }
                                    ui.add_space(8.0);
                                    ui.label(
                                        eframe::egui::RichText::new(language.tr(
                                            "设备端宏仅包含按键、摇杆、L2/R2、触摸和时间信息；不包含灯效、震动、自适应扳机、条件或动作传感器。",
                                            "Device macros contain only buttons, sticks, L2/R2, touch, and timing. They never contain lighting, rumble, adaptive-trigger, conditional, or motion-sensor actions.",
                                        ))
                                        .color(COLOR_TEXT_MUTED),
                                    );
                                });
                            }
                        });
                });
            if refresh_mapping {
                self.refresh_button_mapping();
            }
            if apply_mapping {
                self.apply_button_mapping(self.selected_button_mapping.clone());
            }
            if reset_mapping {
                self.reset_button_mapping();
            }
            if refresh_macros {
                self.refresh_device_macros();
            }
            if apply_macros {
                self.apply_device_macros();
            }
        }

        if self.current_tab == AppTab::Flasher {
            let mut ota_switch = None;
            let mut choose_local_ota = false;
            let mut start_local_ota = false;
            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame::new().fill(COLOR_APP_BG).inner_margin(20))
                .show(ctx, |ui| {
            eframe::egui::ScrollArea::vertical()
                .id_salt("flasher_page_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
            page_heading(
                ui,
                language.tr("设备维护", "Device maintenance"),
                language.tr("固件刷写", "Firmware flasher"),
                language.tr(
                    "选择经过验证的固件与目标串口，刷写前会再次确认 ISP 模式。",
                    "Choose verified firmware and the target port. ISP mode is confirmed before flashing.",
                ),
            );
            ui.add_space(10.0);
            surface_frame().show(ui, |ui| {
            eframe::egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([18.0, 12.0])
                .show(ui, |ui| {
                    ui.label(language.tr("固件来源", "Firmware source"));
                    let previous_firmware_mode = self.firmware_mode;
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            for mode in [
                                FirmwareMode::Online,
                                FirmwareMode::LocalZip,
                                FirmwareMode::LocalDirectory,
                            ] {
                                ui.radio_value(&mut self.firmware_mode, mode, mode.tr(language));
                            }
                        });
                    });
                    if self.firmware_mode != previous_firmware_mode {
                        self.local_firmware = None;
                        self.local_firmware_path = None;
                    }
                    ui.end_row();

                    ui.label(language.tr("固件版本", "Firmware"));
                    ui.horizontal(|ui| match self.firmware_mode {
                        FirmwareMode::Online => {
                            let visible_release_indices =
                                gui_release_indices(&self.releases, self.show_advanced_firmware);
                            if !visible_release_indices.contains(&self.selected_release) {
                                self.selected_release =
                                    preferred_release_index(&self.releases).unwrap_or(0);
                            }
                            if self.loading_releases {
                                ui.spinner();
                            }
                            let selected_text = self
                                .selected_release()
                                .map(|release| gui_release_label(&release, language))
                                .unwrap_or_else(|| {
                                    language.tr("暂无可用固件", "Unavailable").to_owned()
                                });
                            ui.add_enabled_ui(!busy && !self.releases.is_empty(), |ui| {
                                eframe::egui::ComboBox::from_id_salt("release_combo")
                                    .selected_text(selected_text)
                                    .width(390.0)
                                    .show_ui(ui, |ui| {
                                        for index in visible_release_indices.iter().copied() {
                                            let release = &self.releases[index];
                                            ui.selectable_value(
                                                &mut self.selected_release,
                                                index,
                                                gui_release_label(release, language),
                                            );
                                        }
                                    });
                            });
                            if ui
                                .add_enabled(
                                    !busy,
                                    eframe::egui::Button::new(language.tr("刷新", "Refresh")),
                                )
                                .clicked()
                            {
                                self.refresh_releases();
                            }
                            ui.checkbox(
                                &mut self.show_advanced_firmware,
                                language.tr(
                                    "高级固件（显示诊断版）",
                                    "Advanced firmware (show Diagnostic)",
                                ),
                            );
                        }
                        FirmwareMode::LocalZip => {
                            if ui
                                .add_enabled(
                                    !busy,
                                    eframe::egui::Button::new(
                                        language.tr("选择固件 ZIP...", "Choose firmware ZIP..."),
                                    ),
                                )
                                .clicked()
                            {
                                self.choose_local_zip();
                            }
                            ui.label(
                                self.local_firmware
                                    .as_ref()
                                    .map(|set| set.label.as_str())
                                    .unwrap_or(language.tr("尚未选择", "Not selected")),
                            );
                            if let Some(path) = &self.local_firmware_path {
                                ui.monospace(path.display().to_string());
                            }
                        }
                        FirmwareMode::LocalDirectory => {
                            if ui
                                .add_enabled(
                                    !busy,
                                    eframe::egui::Button::new(
                                        language.tr(
                                            "选择完整固件目录...",
                                            "Choose firmware directory...",
                                        ),
                                    ),
                                )
                                .clicked()
                            {
                                self.choose_local_directory();
                            }
                            ui.label(
                                self.local_firmware
                                    .as_ref()
                                    .map(|set| set.label.as_str())
                                    .unwrap_or(language.tr("尚未选择", "Not selected")),
                            );
                            if let Some(path) = &self.local_firmware_path {
                                ui.monospace(path.display().to_string());
                            }
                        }
                    });
                    ui.end_row();

                    ui.label(language.tr("设备固件信息", "Device firmware"));
                    ui.horizontal(|ui| {
                        if self.loading_firmware_devices {
                            ui.spinner();
                        }
                        ui.label(self.firmware_device_text());
                        if ui
                            .add_enabled(
                                !busy && !self.loading_firmware_devices,
                                eframe::egui::Button::new(language.tr("重新读取", "Read again")),
                            )
                            .clicked()
                        {
                            self.refresh_firmware_devices();
                        }
                    });
                    ui.end_row();

                    ui.label(language.tr("一键诊断", "Diagnostics"));
                    ui.horizontal(|ui| {
                        if self.loading_diagnostics {
                            ui.spinner();
                        }
                        ui.label(self.diagnostic_status_text());
                        if ui
                            .add_enabled(
                                !busy && !self.loading_diagnostics,
                                eframe::egui::Button::new(
                                    language.tr("运行一键诊断", "Run diagnostics"),
                                ),
                            )
                            .clicked()
                        {
                            self.start_diagnostics();
                        }
                        if ui
                            .add_enabled(
                                !self.loading_diagnostics
                                    && (!self.runtime_diagnostics.is_empty()
                                        || !self.devices.is_empty()
                                        || self.diagnostics_error.is_some()),
                                eframe::egui::Button::new(language.tr("查看详情", "Details")),
                            )
                            .clicked()
                        {
                            self.show_diagnostics_window = true;
                        }
                    });
                    ui.end_row();

                    ui.label(language.tr("串口设备状态", "Serial device status"));
                    ui.horizontal(|ui| {
                        let (text, color) = self.device_status_text();
                        ui.colored_label(color, text);
                        if ui
                            .add_enabled(
                                !busy,
                                eframe::egui::Button::new(language.tr("重新检测", "Detect again")),
                            )
                            .clicked()
                        {
                            self.refresh_devices();
                        }
                        if ui
                            .add_enabled(
                                !busy && self.has_ch340(),
                                eframe::egui::Button::new(
                                    language.tr("安装/修复驱动", "Install/repair driver"),
                                ),
                            )
                            .clicked()
                        {
                            self.show_driver_dialog = true;
                        }
                    });
                    ui.end_row();

                    ui.label(language.tr("串口", "COM port"));
                    let ports = self.usable_ports();
                    ui.add_enabled_ui(!busy && !ports.is_empty(), |ui| {
                        eframe::egui::ComboBox::from_id_salt("port_combo")
                            .selected_text(
                                self.selected_port
                                    .as_deref()
                                    .unwrap_or(language.tr("没有可用串口", "No usable port")),
                            )
                            .show_ui(ui, |ui| {
                                for port in ports {
                                    ui.selectable_value(
                                        &mut self.selected_port,
                                        Some(port.clone()),
                                        port,
                                    );
                                }
                            });
                    });
                    ui.end_row();

                    ui.label(language.tr("刷写速度", "Baud rate"));
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.radio_value(
                                &mut self.baud,
                                460_800,
                                language.tr("460800（推荐/快速）", "460800 (recommended/fast)"),
                            );
                            ui.radio_value(
                                &mut self.baud,
                                115_200,
                                language.tr("115200（兼容）", "115200 (compatibility)"),
                            );
                        });
                    });
                    ui.end_row();
                });
            });

            ui.add_space(12.0);
            surface_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(language.tr("签名 OTA 升级", "Signed OTA update"));
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            ui.monospace(
                                self.current_build_profile()
                                    .map(BuildProfile::label)
                                    .unwrap_or(language.tr("未识别设备", "Device not detected")),
                            );
                        },
                    );
                });
                ui.label(
                    eframe::egui::RichText::new(language.tr(
                        "无需进入 UART ISP，可在常用版与诊断版之间安全切换。升级会校验 Release、ZIP、镜像哈希和 P-256 签名，并由 A/B 分区负责试运行和失败回滚。",
                        "Update without UART ISP and safely switch between Standard and Diagnostic. The tool verifies the Release, ZIP, image hashes, and P-256 signature; A/B slots provide trial boot and rollback.",
                    ))
                    .color(COLOR_TEXT_MUTED),
                );
                ui.add_space(6.0);
                notice(
                    ui,
                    NoticeTone::Warning,
                    language.tr("OTA 安全提示", "OTA safety"),
                    language.tr(
                        "升级期间会停止测试输出并短暂断开设备。请保持 USB 供电；Boot2、分区表或两槽损坏时仍需 UART 完整刷写。",
                        "Test output stops and the device disconnects briefly during update. Keep USB powered; damaged Boot2, partition tables, or both slots still require a full UART flash.",
                    ),
                );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    let current = self.current_build_profile();
                    if ui
                        .add_enabled(
                            self.busy.is_none()
                                && !self.guided_test.active
                                && !self.device_test_audio_busy
                                && !self.loading_diagnostics
                                && current != Some(BuildProfile::Diagnostic)
                                && self
                                    .latest_release_for_profile(BuildProfile::Diagnostic)
                                    .is_some(),
                            primary_button(language.tr(
                                "OTA 进入诊断版",
                                "OTA to Diagnostic",
                            )),
                        )
                        .clicked()
                    {
                        ota_switch = Some(BuildProfile::Diagnostic);
                    }
                    if ui
                        .add_enabled(
                            self.busy.is_none()
                                && !self.guided_test.active
                                && !self.device_test_audio_busy
                                && !self.loading_diagnostics
                                && current == Some(BuildProfile::Diagnostic)
                                && self
                                    .latest_release_for_profile(BuildProfile::Standard)
                                    .is_some(),
                            eframe::egui::Button::new(language.tr(
                                "OTA 恢复常用版",
                                "OTA to Standard",
                            )),
                        )
                        .clicked()
                    {
                        ota_switch = Some(BuildProfile::Standard);
                    }
                    if ui
                        .add_enabled(
                            self.busy.is_none() && !self.loading_firmware_devices,
                            eframe::egui::Button::new(language.tr(
                                "重新读取设备模式",
                                "Read device profile again",
                            )),
                        )
                        .clicked()
                    {
                        self.refresh_firmware_devices();
                    }
                });
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.busy.is_none(),
                            eframe::egui::Button::new(language.tr(
                                "选择本地签名 OTA ZIP…",
                                "Choose signed local OTA ZIP…",
                            )),
                        )
                        .clicked()
                    {
                        choose_local_ota = true;
                    }
                    if let Some(info) = &self.local_ota_info {
                        ui.monospace(format!(
                            "{} · {} · {} · {} bytes · key {}",
                            info.version,
                            info.profile.localized_label(language),
                            info.channel,
                            info.image_size,
                            info.key_id
                        ));
                    } else {
                        ui.label(language.tr(
                            "尚未选择本地 OTA 包",
                            "No local OTA package selected",
                        ));
                    }
                });
                if let Some(path) = &self.local_ota_path {
                    ui.monospace(path.display().to_string());
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.busy.is_none()
                                && !self.guided_test.active
                                && !self.device_test_audio_busy
                                && !self.loading_diagnostics
                                && self.local_ota_info.is_some(),
                            primary_button(language.tr(
                                "确认并开始本地 OTA",
                                "Confirm and start local OTA",
                            )),
                        )
                        .clicked()
                    {
                        start_local_ota = true;
                    }
                    ui.label(language.tr(
                        "只接受包含 .bin.ota 与 .ota.json 且电脑端 P-256 验签通过的 AIM61 HS ZIP。",
                        "Only AIM61 HS ZIPs containing .bin.ota and .ota.json and passing PC-side P-256 verification are accepted.",
                    ));
                });
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let can_flash =
                    !busy && self.selected_firmware().is_some() && self.selected_port.is_some();
                if ui
                    .add_enabled(
                        can_flash,
                        eframe::egui::Button::new(
                            eframe::egui::RichText::new(
                                language.tr("下载并刷写", "Download & Flash"),
                            )
                            .strong()
                            .color(COLOR_TEXT_PRIMARY),
                        )
                        .fill(COLOR_ACCENT_SOFT)
                        .stroke(eframe::egui::Stroke::new(1.5_f32, COLOR_ACCENT))
                        .min_size([260.0, 42.0].into()),
                    )
                    .clicked()
                {
                    self.show_isp_dialog = true;
                }
                if let Some(operation) = &self.busy {
                    ui.spinner();
                    ui.label(operation);
                }
            });

            ui.add_space(10.0);
            notice(
                ui,
                status_tone(&self.status),
                language.tr("当前状态", "Current status"),
                &self.status,
            );
            ui.add_space(12.0);
            surface_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(language.tr("操作日志", "Activity log"));
                    ui.with_layout(
                        eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                        |ui| {
                            if ui.button(language.tr("复制全部", "Copy all")).clicked() {
                                ui.ctx().copy_text(self.log.clone());
                            }
                        },
                    );
                });
                ui.separator();
                eframe::egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .max_height(160.0)
                    .show(ui, |ui| {
                        ui.add(
                            eframe::egui::Label::new(
                                eframe::egui::RichText::new(&self.log)
                                    .color(COLOR_TEXT_MUTED)
                                    .monospace(),
                            )
                            .selectable(true)
                            .wrap(),
                        );
                    });
            });
            });
            });
            if let Some(profile) = ota_switch {
                self.start_profile_ota(profile);
            }
            if choose_local_ota {
                self.choose_local_ota_zip();
            }
            if start_local_ota {
                self.start_local_ota();
            }
        }

        if self.current_tab == AppTab::TestCenter {
            ctx.request_repaint_after(Duration::from_millis(16));
            let mut apply_output = false;
            let mut stop_output = false;
            let mut reconnect = false;
            let mut tone = None;
            let mut mic_test = false;
            let mut save_microphone_wav = false;
            let mut start_controller_tone = None;
            let mut stop_controller_tone = false;
            let mut calibration_step = None;
            let mut export_complete_report = false;
            let input = self.device_test_input.clone();
            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame::new().fill(COLOR_APP_BG).inner_margin(20))
                .show(ctx, |ui| {
                eframe::egui::ScrollArea::vertical()
                    .id_salt("test_center_page_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                page_heading(
                    ui,
                    language.tr("实时验证", "Live validation"),
                    language.tr(
                        "DS5 手柄功能测试中心",
                        "DS5 controller test center",
                    ),
                    language.tr(
                        "实时验证 DualSense 的输入、输出、传感器和 USB 音频链路。",
                        "Validate DualSense input, output, sensors and USB audio in real time.",
                    ),
                );
                ui.add_space(8.0);
                let connected = self.device_test_connected;
                let worker_active = self.device_test_session.is_some();
                let benchmark_running = self.device_debug_started.is_some();
                notice(
                    ui,
                    if connected {
                        NoticeTone::Success
                    } else if worker_active {
                        NoticeTone::Info
                    } else {
                        status_tone(&self.device_test_status)
                    },
                    if connected {
                        language.tr("设备已连接", "Device connected")
                    } else if worker_active {
                        language.tr("正在连接设备", "Connecting device")
                    } else {
                        language.tr("设备未连接", "Device not connected")
                    },
                    &self.device_test_status,
                );
                if benchmark_running {
                    ui.add_space(8.0);
                    notice(
                        ui,
                        NoticeTone::Warning,
                        language.tr("压力测试正在运行", "Stress test is running"),
                        language.tr(
                            "手动震动、灯效、扳机和 1 kHz 声音控制已锁定。点击“全部停止并复位”会同时提前结束压力测试。",
                            "Manual rumble, light, trigger and 1 kHz controls are locked. Stop all and reset also ends the stress test early.",
                        ),
                    );
                }
                if !worker_active
                    && ui
                        .add(primary_button(language.tr("重新连接", "Reconnect")))
                        .clicked()
                {
                    reconnect = true;
                }
                ui.add_space(8.0);
                notice(
                    ui,
                    NoticeTone::Warning,
                    language.tr("避免设备冲突", "Avoid device conflicts"),
                    language.tr(
                        "请关闭 Steam、DS4Windows 和浏览器手柄测试页。关闭程序时会自动复位所有输出。",
                        "Close Steam, DS4Windows and other controller tools. All outputs reset automatically when the app closes.",
                    ),
                );
                ui.add_space(12.0);

                ui.vertical(|ui| {
                    surface_frame().show(ui, |ui| {
                    ui.heading(language.tr("实时输入", "Live input"));
                    controller_input_diagram(ui, &input, language);
                    ui.add_space(6.0);
                    eframe::egui::Grid::new("device_test_axes")
                        .num_columns(4)
                        .spacing([12.0, 5.0])
                        .show(ui, |ui| {
                            input_progress(ui, "LX", input.lx);
                            input_progress(ui, "LY", input.ly);
                            ui.end_row();
                            input_progress(ui, "RX", input.rx);
                            input_progress(ui, "RY", input.ry);
                            ui.end_row();
                            input_progress(ui, "L2", input.l2);
                            input_progress(ui, "R2", input.r2);
                            ui.end_row();
                        });

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.group(|ui| {
                            ui.strong(language.tr("六轴传感器", "Motion sensors"));
                            eframe::egui::Grid::new("device_test_motion").show(ui, |ui| {
                                ui.label("Gyro X/Y/Z");
                                ui.monospace(format!(
                                    "{} / {} / {}",
                                    input.gyro_x, input.gyro_y, input.gyro_z
                                ));
                                ui.end_row();
                                ui.label("Accel X/Y/Z");
                                ui.monospace(format!(
                                    "{} / {} / {}",
                                    input.accel_x, input.accel_y, input.accel_z
                                ));
                                ui.end_row();
                                if let Some(gyro) = input.gyro_dps {
                                    ui.label("Gyro °/s");
                                    ui.monospace(format!(
                                        "{:.2} / {:.2} / {:.2}",
                                        gyro[0], gyro[1], gyro[2]
                                    ));
                                    ui.end_row();
                                }
                                if let Some(accel) = input.accel_g {
                                    ui.label("Accel g");
                                    ui.monospace(format!(
                                        "{:.3} / {:.3} / {:.3}",
                                        accel[0], accel[1], accel[2]
                                    ));
                                    ui.end_row();
                                }
                            });
                            if let Some(calibration) = &self.controller_motion_calibration {
                                let source = match calibration.source {
                                    controller_calibration::CalibrationSource::ControllerFactory => {
                                        language.tr("手柄原厂校准", "Controller factory calibration")
                                    }
                                    controller_calibration::CalibrationSource::M61CompatibilityFallback => {
                                        language.tr("M61 兼容占位校准", "M61 compatibility fallback")
                                    }
                                    controller_calibration::CalibrationSource::NominalFallback => {
                                        language.tr("标称比例回退", "Nominal scaling fallback")
                                    }
                                };
                                ui.label(
                                    eframe::egui::RichText::new(source).color(
                                        if calibration.factory_data_valid {
                                            COLOR_SUCCESS
                                        } else {
                                            COLOR_WARNING
                                        },
                                    ),
                                );
                            }
                        });
                        ui.group(|ui| {
                            ui.strong(language.tr("触摸板", "Touchpad"));
                            for (index, touch) in input.touch.iter().enumerate() {
                                ui.label(if touch.active {
                                    format!("#{} ID {}: {}, {}", index + 1, touch.id, touch.x, touch.y)
                                } else {
                                    format!("#{}: {}", index + 1, language.tr("未触摸", "inactive"))
                                });
                            }
                        });
                        ui.group(|ui| {
                            ui.strong(language.tr("状态", "Status"));
                            ui.label(match input.battery_percent {
                                Some(value) => format!(
                                    "{}: {value}%{}",
                                    language.tr("电量", "Battery"),
                                    if input.battery_charging {
                                        language.tr("（充电中）", " (charging)")
                                    } else if input.battery_cable_connected {
                                        language.tr("（已接电源）", " (external power)")
                                    } else {
                                        ""
                                    },
                                ),
                                None => language.tr("电量：未知", "Battery: unknown").to_owned(),
                            });
                            if input.battery_error {
                                ui.colored_label(
                                    COLOR_WARNING,
                                    language.tr("电池状态异常", "Battery status error"),
                                );
                            }
                            ui.label(format!(
                                "{}: {} · {}: {}",
                                language.tr("3.5 mm 耳机", "3.5 mm headphone"),
                                language.tr(
                                    if input.headphone_connected { "已连接" } else { "未连接" },
                                    if input.headphone_connected { "connected" } else { "not connected" },
                                ),
                                language.tr("耳麦麦克风", "Headset microphone"),
                                language.tr(
                                    if input.headset_microphone_connected { "已连接" } else { "未连接" },
                                    if input.headset_microphone_connected { "connected" } else { "not connected" },
                                ),
                            ));
                            ui.label(format!("{}: {}", language.tr("报告数", "Reports"), input.report_count));
                            ui.label(format!(
                                "{}: {:.1} Hz",
                                language.tr("输入报告频率", "Input report rate"),
                                input.report_rate_hz
                            ));
                        });
                    });
                    });

                    ui.add_space(12.0);
                    surface_frame().show(ui, |ui| {
                        ui.heading(language.tr(
                            "摇杆质量分析",
                            "Stick quality analyzer",
                        ));
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "松开摇杆采集中心，然后开启轨迹采集并将两个摇杆沿外圈缓慢转动 3–5 圈。圆度误差采用 dualshock-tools 的 48 方向 RMS 定义。",
                                "Capture center with both sticks released, then record while rotating both sticks slowly around the rim 3–5 times. Circularity uses dualshock-tools' 48-direction RMS definition.",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            let capture_label = if self.controller_analyzer.active {
                                language.tr("暂停轨迹采集", "Pause recording")
                            } else {
                                language.tr("开始轨迹采集", "Start recording")
                            };
                            if ui
                                .add_enabled(connected, primary_button(capture_label))
                                .clicked()
                            {
                                self.controller_analyzer.active = !self.controller_analyzer.active;
                            }
                            if ui
                                .add_enabled(
                                    connected,
                                    eframe::egui::Button::new(language.tr(
                                        "采集松手中心",
                                        "Capture released center",
                                    )),
                                )
                                .clicked()
                            {
                                self.controller_analyzer.begin_center_capture();
                            }
                            if ui
                                .button(language.tr("清空分析", "Reset analysis"))
                                .clicked()
                            {
                                self.controller_analyzer.reset();
                            }
                            ui.label(if self.controller_analyzer.active {
                                language.tr("● 正在采集", "● Recording")
                            } else {
                                language.tr("○ 已暂停", "○ Paused")
                            });
                        });
                        if let Some(progress) = self.controller_analyzer.center_capture_progress() {
                            ui.add(
                                eframe::egui::ProgressBar::new(progress)
                                    .desired_width(320.0)
                                    .text(language.tr(
                                        "保持双摇杆松手不动",
                                        "Keep both sticks released",
                                    )),
                            );
                        }
                        ui.add_space(8.0);
                        ui.columns(2, |columns| {
                            stick_analysis_dial(
                                &mut columns[0],
                                language.tr("左摇杆", "Left stick"),
                                input.lx,
                                input.ly,
                                &self.controller_analyzer.left,
                            );
                            stick_analysis_dial(
                                &mut columns[1],
                                language.tr("右摇杆", "Right stick"),
                                input.rx,
                                input.ry,
                                &self.controller_analyzer.right,
                            );
                        });
                        ui.columns(2, |columns| {
                            stick_analysis_metrics(
                                &mut columns[0],
                                language,
                                &self.controller_analyzer.left,
                                self.controller_analyzer.left_center,
                                self.controller_analyzer.left_assessment(),
                            );
                            stick_analysis_metrics(
                                &mut columns[1],
                                language,
                                &self.controller_analyzer.right,
                                self.controller_analyzer.right_center,
                                self.controller_analyzer.right_assessment(),
                            );
                        });

                        ui.add_space(8.0);
                        ui.collapsing(
                            language.tr(
                                "高级：写入手柄永久摇杆校准",
                                "Advanced: permanent controller stick calibration",
                            ),
                            |ui| {
                                let analysis_complete =
                                    self.controller_analyzer.read_only_analysis_complete();
                                notice(
                                    ui,
                                    NoticeTone::Warning,
                                    language.tr("会修改手柄", "Writes to the controller"),
                                    language.tr(
                                        "此操作会把中心或范围参数永久写入已连接的 DualSense。开始校准时会自动停止声音、震动、灯效和扳机输出。请先完成上方只读分析；校准过程中保持 USB 和蓝牙供电，不要关闭程序。旧版 M61 固件不支持此通道。",
                                        "This permanently writes center or range data to the connected DualSense. Audio, rumble, light and trigger outputs stop automatically before calibration. Run the read-only analyzer first; keep USB and Bluetooth powered and do not close the app during calibration. Older M61 firmware does not support this bridge.",
                                    ),
                                );
                                if !analysis_complete {
                                    notice(
                                        ui,
                                        NoticeTone::Info,
                                        language.tr(
                                            "先完成只读分析",
                                            "Complete read-only analysis first",
                                        ),
                                        &match self.language {
                                            Language::ZhCn => format!(
                                                "请先采集松手中心，并让左右摇杆的方向覆盖都达到至少 {:.0}%。检测无需通过，只需取得足够样本。",
                                                self.controller_analyzer.required_coverage_percent(),
                                            ),
                                            Language::En => format!(
                                                "Capture released center and reach at least {:.0}% direction coverage on both sticks. The result need not pass; it only needs enough samples.",
                                                self.controller_analyzer.required_coverage_percent(),
                                            ),
                                        },
                                    );
                                }
                                ui.add_enabled_ui(analysis_complete, |ui| {
                                    ui.checkbox(
                                        &mut self.calibration_confirmed,
                                        language.tr(
                                            "我确认已了解风险，并只对确有漂移或范围异常的手柄执行",
                                            "I understand the risk and will use this only for a controller with verified drift or range errors",
                                        ),
                                    );
                                });
                                ui.label(
                                    eframe::egui::RichText::new(&self.calibration_status)
                                        .color(if self.calibration_busy {
                                            COLOR_ACCENT_HOVER
                                        } else {
                                            COLOR_TEXT_PRIMARY
                                        }),
                                );
                                let ready = connected
                                    && analysis_complete
                                    && self.calibration_confirmed
                                    && !self.calibration_busy;
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add_enabled(
                                            ready && self.calibration_state == CalibrationUiState::Idle,
                                            eframe::egui::Button::new(language.tr(
                                                "1. 开始中心校准",
                                                "1. Begin center calibration",
                                            )),
                                        )
                                        .clicked()
                                    {
                                        calibration_step = Some(device_test::CalibrationStep::CenterBegin);
                                    }
                                    if ui
                                        .add_enabled(
                                            ready && self.calibration_state == CalibrationUiState::Center,
                                            eframe::egui::Button::new(language.tr(
                                                "2. 松手采样（可多次）",
                                                "2. Sample released sticks (repeatable)",
                                            )),
                                        )
                                        .clicked()
                                    {
                                        calibration_step = Some(device_test::CalibrationStep::CenterSample);
                                    }
                                    if ui
                                        .add_enabled(
                                            ready && self.calibration_state == CalibrationUiState::Center,
                                            eframe::egui::Button::new(language.tr(
                                                "3. 写入中心校准",
                                                "3. Commit center calibration",
                                            )),
                                        )
                                        .clicked()
                                    {
                                        calibration_step = Some(device_test::CalibrationStep::CenterCommit);
                                    }
                                });
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add_enabled(
                                            ready && self.calibration_state == CalibrationUiState::Idle,
                                            eframe::egui::Button::new(language.tr(
                                                "1. 开始范围校准",
                                                "1. Begin range calibration",
                                            )),
                                        )
                                        .clicked()
                                    {
                                        calibration_step = Some(device_test::CalibrationStep::RangeBegin);
                                    }
                                    ui.label(language.tr(
                                        "开始后将两个摇杆贴外圈缓慢旋转 3–5 圈",
                                        "After beginning, rotate both sticks along the rim 3–5 times",
                                    ));
                                    if ui
                                        .add_enabled(
                                            ready && self.calibration_state == CalibrationUiState::Range,
                                            eframe::egui::Button::new(language.tr(
                                                "2. 写入范围校准",
                                                "2. Commit range calibration",
                                            )),
                                        )
                                        .clicked()
                                    {
                                        calibration_step = Some(device_test::CalibrationStep::RangeCommit);
                                    }
                                });
                            },
                        );
                    });

                    ui.add_space(12.0);
                    surface_frame().show(ui, |ui| {
                    ui.heading(language.tr("输出功能", "Output functions"));
                    ui.columns(2, |columns| {
                        columns[0].group(|ui| {
                            ui.strong(language.tr("震动与灯效", "Rumble and lights"));
                            ui.add(
                                eframe::egui::Slider::new(
                                    &mut self.device_test_output.rumble_left,
                                    0..=200,
                                )
                                .text(language.tr("左侧低频", "Left / low frequency")),
                            );
                            ui.add(
                                eframe::egui::Slider::new(
                                    &mut self.device_test_output.rumble_right,
                                    0..=200,
                                )
                                .text(language.tr("右侧高频", "Right / high frequency")),
                            );
                            ui.checkbox(
                                &mut self.device_test_output.lightbar_enabled,
                                language.tr("启用灯条", "Enable lightbar"),
                            );
                            ui.horizontal(|ui| {
                                ui.label("R/G/B");
                                for value in &mut self.device_test_output.lightbar_rgb {
                                    ui.add(eframe::egui::DragValue::new(value).range(0..=255));
                                }
                            });
                            ui.add(
                                eframe::egui::Slider::new(
                                    &mut self.device_test_output.player_leds,
                                    0..=31,
                                )
                                .text(language.tr("玩家灯位图", "Player LED mask")),
                            );
                            ui.add(
                                eframe::egui::Slider::new(
                                    &mut self.device_test_output.mute_led,
                                    0..=2,
                                )
                                .text(language.tr("静音灯 0/1/2", "Mute LED 0/1/2")),
                            );
                        });
                        columns[1].group(|ui| {
                            ui.strong(language.tr("自适应扳机", "Adaptive triggers"));
                            for (id, label, value, force) in [
                                (
                                    "left_trigger_preset",
                                    "L2",
                                    &mut self.device_test_output.left_trigger,
                                    &mut self.device_test_output.left_trigger_force,
                                ),
                                (
                                    "right_trigger_preset",
                                    "R2",
                                    &mut self.device_test_output.right_trigger,
                                    &mut self.device_test_output.right_trigger_force,
                                ),
                            ] {
                                ui.horizontal(|ui| {
                                    ui.label(label);
                                    eframe::egui::ComboBox::from_id_salt(id)
                                        .selected_text(match *value {
                                            device_test::TriggerPreset::Off => language.tr("关闭", "Off"),
                                            device_test::TriggerPreset::Resistance => language.tr("阻力", "Resistance"),
                                            device_test::TriggerPreset::Weapon => language.tr("扳机", "Weapon"),
                                            device_test::TriggerPreset::Automatic => language.tr("自动扳机", "Automatic"),
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(value, device_test::TriggerPreset::Off, language.tr("关闭", "Off"));
                                            ui.selectable_value(value, device_test::TriggerPreset::Resistance, language.tr("阻力", "Resistance"));
                                            ui.selectable_value(value, device_test::TriggerPreset::Weapon, language.tr("扳机", "Weapon"));
                                            ui.selectable_value(value, device_test::TriggerPreset::Automatic, language.tr("自动扳机", "Automatic"));
                                        });
                                });
                                if *value != device_test::TriggerPreset::Off {
                                    ui.add(
                                        eframe::egui::Slider::new(force, 0..=255)
                                            .text(match language {
                                                Language::ZhCn => format!("{label} 力度"),
                                                Language::En => format!("{label} force"),
                                            }),
                                    );
                                }
                            }
                            notice(
                                ui,
                                NoticeTone::Warning,
                                language.tr("扳机安全提示", "Trigger safety"),
                                language.tr(
                                    "力度范围为 0–255；超过 200 只建议短时验证。引导诊断和压力测试仍固定使用低强度，测试后请点击“全部停止并复位”。",
                                    "Force ranges from 0–255; values above 200 are recommended only for brief checks. Guided diagnostics and stress tests remain fixed at low force. Click Stop all and reset afterwards.",
                                ),
                            );
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.add_enabled(
                            connected && !benchmark_running,
                            primary_button(language.tr("应用输出测试", "Apply output test")),
                        ).clicked() {
                            apply_output = true;
                        }
                        if ui.add(
                            eframe::egui::Button::new(
                                eframe::egui::RichText::new(language.tr(
                                    "全部停止并复位",
                                    "Stop all and reset",
                                ))
                                .color(COLOR_ERROR)
                                .strong(),
                            )
                            .fill(COLOR_ERROR_SOFT)
                            .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_ERROR)),
                        ).clicked() {
                            stop_output = true;
                        }
                    });
                    });

                    ui.add_space(12.0);
                    surface_frame().show(ui, |ui| {
                    ui.heading(language.tr("手柄声音测试", "Controller audio test"));
                    notice(
                        ui,
                        NoticeTone::Info,
                        language.tr("原生 1 kHz 测试", "Native 1 kHz test"),
                        language.tr(
                            "通过与 ds.evua.cc 相同的 DualSense Feature Report 0x80，直接测试手柄扬声器或已插入手柄的耳机；无需更改 Windows 默认音频设备。",
                            "Uses the same DualSense Feature Report 0x80 flow as ds.evua.cc to test the controller speaker or a headset connected to the controller; no Windows default-device change is required.",
                        ),
                    );
                    ui.horizontal(|ui| {
                        let speaker_active = self.device_test_controller_tone
                            == Some(device_test::ControllerAudioTarget::Speaker);
                        let headphone_active = self.device_test_controller_tone
                            == Some(device_test::ControllerAudioTarget::Headphone);
                        if ui.add_enabled(
                            connected && !benchmark_running,
                            eframe::egui::Button::new(if speaker_active {
                                language.tr("停止扬声器", "Stop speaker")
                            } else {
                                language.tr("扬声器 1 kHz", "Speaker 1 kHz")
                            }),
                        ).clicked() {
                            if speaker_active {
                                stop_controller_tone = true;
                            } else {
                                start_controller_tone = Some(device_test::ControllerAudioTarget::Speaker);
                            }
                        }
                        if ui.add_enabled(
                            connected && !benchmark_running,
                            eframe::egui::Button::new(if headphone_active {
                                language.tr("停止耳机", "Stop headphone")
                            } else {
                                language.tr("耳机 1 kHz", "Headphone 1 kHz")
                            }),
                        ).clicked() {
                            if headphone_active {
                                stop_controller_tone = true;
                            } else {
                                start_controller_tone = Some(device_test::ControllerAudioTarget::Headphone);
                            }
                        }
                        if ui.add_enabled(
                            connected
                                && !benchmark_running
                                && self.device_test_controller_tone.is_some(),
                            eframe::egui::Button::new(language.tr("停止 1 kHz", "Stop 1 kHz")),
                        ).clicked() {
                            stop_controller_tone = true;
                        }
                    });
                    });

                    ui.add_space(12.0);
                    surface_frame().show(ui, |ui| {
                    ui.heading(language.tr("USB 音频链路", "USB audio path"));
                    notice(
                        ui,
                        NoticeTone::Info,
                        language.tr("Windows 音频设备", "Windows audio device"),
                        language.tr(
                            "麦克风通过 WASAPI 自动选择 M61/DualSense 输入端点，不依赖 Windows 默认输入。测试音仍由 Windows 默认输出播放；录音与回放结果分别记录。",
                            "The microphone uses WASAPI to select the M61/DualSense input endpoint and does not depend on the Windows default input. Test tones still use the default output; capture and playback results are recorded separately.",
                        ),
                    );
                    ui.horizontal(|ui| {
                        if self.device_test_audio_busy {
                            ui.spinner();
                        }
                        if ui.add_enabled(!self.device_test_audio_busy, eframe::egui::Button::new(language.tr("左声道", "Left tone"))).clicked() {
                            tone = Some(device_test::AudioChannel::Left);
                        }
                        if ui.add_enabled(!self.device_test_audio_busy, eframe::egui::Button::new(language.tr("右声道", "Right tone"))).clicked() {
                            tone = Some(device_test::AudioChannel::Right);
                        }
                        if ui.add_enabled(!self.device_test_audio_busy, eframe::egui::Button::new(language.tr("双声道", "Stereo tone"))).clicked() {
                            tone = Some(device_test::AudioChannel::Both);
                        }
                        if ui.add_enabled(!self.device_test_audio_busy, eframe::egui::Button::new(language.tr("录音 5 秒并回放", "Record 5s and play back"))).clicked() {
                            mic_test = true;
                        }
                        if ui.add_enabled(
                            self.last_microphone_wav.is_some() && !self.device_test_audio_busy,
                            eframe::egui::Button::new(language.tr("保存最近录音…", "Save latest recording…")),
                        ).clicked() {
                            save_microphone_wav = true;
                        }
                    });
                    if let Some(metrics) = &self.last_microphone_metrics {
                        ui.add_space(8.0);
                        notice(
                            ui,
                            if metrics.signal_detected {
                                NoticeTone::Success
                            } else {
                                NoticeTone::Warning
                            },
                            language.tr("最近一次麦克风信号分析", "Latest microphone signal analysis"),
                            &match self.language {
                                Language::ZhCn => format!(
                                    "{} · {} Hz/{} 声道 · RMS {:.2}% · 峰值 {:.2}% · 有效窗口 {} · {}{} · 回放{}",
                                    metrics.capture_endpoint,
                                    metrics.sample_rate_hz,
                                    metrics.channels,
                                    metrics.rms_percent,
                                    metrics.peak_percent,
                                    metrics.active_windows,
                                    if metrics.signal_detected { "检测到有效声音" } else { "信号不足" },
                                    metrics.signal_to_silence_db.map(|value| format!(" · 信噪比 {:.1} dB", value)).unwrap_or_default(),
                                    if metrics.playback_succeeded { "成功" } else { "失败" },
                                ),
                                Language::En => format!(
                                    "{} · {} Hz/{} ch · RMS {:.2}% · peak {:.2}% · {} active windows · {}{} · playback {}",
                                    metrics.capture_endpoint,
                                    metrics.sample_rate_hz,
                                    metrics.channels,
                                    metrics.rms_percent,
                                    metrics.peak_percent,
                                    metrics.active_windows,
                                    if metrics.signal_detected { "signal detected" } else { "insufficient signal" },
                                    metrics.signal_to_silence_db.map(|value| format!(" · SNR {:.1} dB", value)).unwrap_or_default(),
                                    if metrics.playback_succeeded { "passed" } else { "failed" },
                                ),
                            },
                        );
                    }
                    });

                    ui.add_space(12.0);
                    surface_frame().show(ui, |ui| {
                        ui.heading(language.tr("完整测试报告", "Complete test report"));
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "将本页输入、摇杆与校准结果，连同设备调试页的引导测试、延迟、压力负载和 M61 快照集中导出为一个 JSON。未测试项目会明确标记。",
                                "Export this page's input, stick and calibration results together with guided tests, latency, stress load and M61 snapshots from Device Debug as one JSON. Untested items are marked explicitly.",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                        let report_has_data = input.report_count > 0
                            || self.controller_analyzer.left.samples() > 0
                            || self.controller_analyzer.right.samples() > 0
                            || !self.calibration_events.is_empty()
                            || self.last_microphone_metrics.is_some()
                            || self.device_debug_metrics.sample_count > 0
                            || guided_test_has_data(&self.guided_test.phases)
                            || self
                                .runtime_diagnostics
                                .iter()
                                .any(|report| report.snapshot.is_some());
                        let report_waiting = self.loading_diagnostics
                            || self.device_debug_final_snapshot_pending;
                        if report_waiting {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(language.tr(
                                    "正在完成运行快照，请稍候。",
                                    "Finishing the runtime snapshot; please wait.",
                                ));
                            });
                        }
                        if ui
                            .add_enabled(
                                report_has_data && !report_waiting,
                                primary_button(language.tr(
                                    "导出完整测试报告 JSON",
                                    "Export complete test report JSON",
                                )),
                            )
                            .clicked()
                        {
                            export_complete_report = true;
                        }
                    });
                });
                });
            });

            if export_complete_report {
                self.export_debug_report();
            }
            if reconnect {
                self.ensure_device_session();
            }
            if apply_output {
                if let Some(session) = &self.device_test_session {
                    if let Err(error) = session.set_output(self.device_test_output.clone()) {
                        self.device_test_status = format!("{error:#}");
                    }
                }
            }
            if stop_output {
                if self.device_debug_started.is_some() {
                    self.stop_debug_benchmark(false);
                }
                self.reset_test_outputs();
            }
            if let Some(target) = start_controller_tone {
                if let Some(session) = &self.device_test_session {
                    if let Err(error) = session.start_controller_tone(target) {
                        self.device_test_status = format!("{error:#}");
                    }
                }
            } else if stop_controller_tone {
                if let Some(session) = &self.device_test_session {
                    if let Err(error) = session.stop_controller_tone() {
                        self.device_test_status = format!("{error:#}");
                    }
                }
            }
            if let Some(step) = calibration_step {
                if let Some(session) = &self.device_test_session {
                    match session.calibrate(step) {
                        Ok(()) => {
                            if matches!(
                                step,
                                device_test::CalibrationStep::CenterBegin
                                    | device_test::CalibrationStep::RangeBegin
                            ) {
                                self.controller_analysis_checkpoints.push(
                                    ControllerAnalysisCheckpoint {
                                        captured_at_unix_ms: diagnostics::now_unix_ms(),
                                        stage: "beforeCalibration",
                                        analysis: self.controller_analyzer.report(),
                                    },
                                );
                            }
                            self.calibration_busy = true;
                            self.calibration_status = self
                                .language
                                .tr(
                                    "正在等待手柄确认校准命令...",
                                    "Waiting for the controller to acknowledge calibration...",
                                )
                                .to_owned();
                        }
                        Err(error) => {
                            self.calibration_busy = false;
                            self.calibration_status = format!("{error:#}");
                        }
                    }
                }
            }
            if let Some(channel) = tone {
                self.start_audio_test(channel);
            }
            if mic_test {
                self.start_microphone_test(5);
            }
            if save_microphone_wav {
                self.save_microphone_wav();
            }
        }

        if self.current_tab == AppTab::DeviceDebug {
            ctx.request_repaint_after(Duration::from_millis(100));
            let mut start_benchmark = false;
            let mut stop_benchmark = false;
            let mut capture_snapshot = false;
            let mut export_report = false;
            let mut guide_start = false;
            let mut guide_cancel = false;
            let mut guide_signal = false;
            let mut guide_retest = false;
            let mut guide_result = None;
            let mut ota_switch = None;
            let mut refresh_polling_rate = false;
            let mut apply_polling_rate = None;
            let metrics = self.device_debug_metrics.clone();
            let running = self.device_debug_started.is_some();
            let extreme_duration_blocked = self.device_debug_stress_enabled
                && self.device_debug_stress_rate_hz == 50
                && self.device_debug_duration_secs > 900;
            let elapsed_secs = self
                .device_debug_started
                .map_or(metrics.elapsed_ms as f32 / 1000.0, |started| {
                    started.elapsed().as_secs_f32()
                });

            eframe::egui::CentralPanel::default()
                .frame(eframe::egui::Frame::new().fill(COLOR_APP_BG).inner_margin(20))
                .show(ctx, |ui| {
                    eframe::egui::ScrollArea::vertical()
                        .id_salt("device_debug_page_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                    page_heading(
                        ui,
                        language.tr("工程诊断", "Engineering diagnostics"),
                        language.tr("设备调试与性能分析", "Device debug and performance"),
                        language.tr(
                            "同时测量 M61 内部蓝牙输入到 USB 完成的转发延迟，以及 Windows 收到 HID 报告的频率、间隔分布与抖动。",
                            "Measure the M61 Bluetooth-input-to-USB-completion bridge latency together with Windows HID arrival rate, interval distribution and jitter.",
                        ),
                    );
                    ui.add_space(10.0);
                    notice(
                        ui,
                        NoticeTone::Info,
                        language.tr("测量边界", "Measurement scope"),
                        language.tr(
                            "这里测得的是 USB HID 报告到达 Windows 的调度间隔，不是从按下按键到屏幕显示的绝对端到端延迟。",
                            "These values are Windows USB HID arrival intervals, not absolute button-to-display end-to-end latency.",
                        ),
                    );
                    ui.add_space(12.0);

                    surface_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading(language.tr("固件运行模式", "Firmware runtime profile"));
                            ui.with_layout(
                                eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                                |ui| {
                                    ui.monospace(
                                        self.current_build_profile()
                                            .map(BuildProfile::label)
                                            .unwrap_or(language.tr("未知", "Unknown")),
                                    );
                                },
                            );
                        });
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "常用版关闭运行诊断采样以获得最低开销；诊断版开放 M61 快照和内部转发延迟，并在 0xFD 失效时通过一次性 0xF8 响应自动回退。两者可通过签名 OTA 来回切换。",
                                "Standard disables runtime diagnostic sampling for minimum overhead. Diagnostic enables M61 snapshots and internal bridge latency, with a one-shot 0xF8 fallback when 0xFD fails. Signed OTA can switch both ways.",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                        ui.add_space(6.0);
                        notice(
                            ui,
                            NoticeTone::Warning,
                            language.tr("OTA 安全提示", "OTA safety"),
                            language.tr(
                                "切换期间会停止所有测试输出并暂时断开手柄接口。请保持 USB 供电，直到设备重新连接。",
                                "All test outputs stop and the gamepad interface disconnects briefly. Keep USB powered until the device reconnects.",
                            ),
                        );
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            let current = self.current_build_profile();
                            if ui
                                .add_enabled(
                                    self.busy.is_none()
                                        && !running
                                        && !self.guided_test.active
                                        && !self.device_test_audio_busy
                                        && !self.loading_diagnostics
                                        && current != Some(BuildProfile::Diagnostic)
                                        && self.latest_release_for_profile(BuildProfile::Diagnostic).is_some(),
                                    primary_button(language.tr(
                                        "进入诊断模式（OTA）",
                                        "Enter Diagnostic mode (OTA)",
                                    )),
                                )
                                .clicked()
                            {
                                ota_switch = Some(BuildProfile::Diagnostic);
                            }
                            if ui
                                .add_enabled(
                                    self.busy.is_none()
                                        && !running
                                        && !self.guided_test.active
                                        && !self.device_test_audio_busy
                                        && !self.loading_diagnostics
                                        && current == Some(BuildProfile::Diagnostic)
                                        && self.latest_release_for_profile(BuildProfile::Standard).is_some(),
                                    eframe::egui::Button::new(language.tr(
                                        "恢复常用版（OTA）",
                                        "Restore Standard (OTA)",
                                    )),
                                )
                                .clicked()
                            {
                                ota_switch = Some(BuildProfile::Standard);
                            }
                            if ui
                                .add_enabled(
                                    self.busy.is_none() && !self.loading_firmware_devices,
                                    eframe::egui::Button::new(language.tr(
                                        "重新读取模式",
                                        "Read profile again",
                                    )),
                                )
                                .clicked()
                            {
                                self.refresh_firmware_devices();
                            }
                        });
                    });

                    ui.add_space(12.0);

                    // The editor lives on the dedicated Mapping page.  Keep
                    // the old block excluded while the surrounding debug
                    // layout remains otherwise unchanged.
                    #[cfg(any())]
                    surface_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading(language.tr("19 控件按键映射", "19-control button mapping"));
                            ui.with_layout(
                                eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                                |ui| {
                                    if self.loading_button_mapping || self.applying_button_mapping {
                                        ui.spinner();
                                    }
                                    ui.monospace(if self
                                        .device_button_mapping
                                        .as_ref()
                                        .is_some_and(device_config::ButtonMapping::is_identity)
                                    {
                                        language.tr("默认一对一", "Default one-to-one")
                                    } else if self.device_button_mapping.is_some() {
                                        language.tr("已自定义", "Customized")
                                    } else {
                                        language.tr("未读取", "Not read")
                                    });
                                },
                            );
                        });
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "每个左侧物理输入可映射为一个目标输入，覆盖 15 个按键和方向键四个方向。允许多个输入映射到同一目标。仅使用新版 19 控件协议，不读取旧 15 键配置。",
                                "Map each physical input on the left to one target input. This covers 15 buttons plus all four D-pad directions. Multiple inputs may share one target. Only the new 19-control protocol is supported; legacy 15-button data is not read.",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                        ui.add_space(8.0);

                        let mapping_enabled = self.device_button_mapping.is_some()
                            && !self.loading_button_mapping
                            && !self.applying_button_mapping
                            && self.busy.is_none()
                            && !running
                            && !self.guided_test.active
                            && !self.device_test_audio_busy;
                        ui.add_enabled_ui(mapping_enabled, |ui| {
                            eframe::egui::Grid::new("button_mapping_grid")
                                .num_columns(4)
                                .spacing([14.0, 6.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    for row in 0..10 {
                                        for index in [row, row + 10] {
                                            if index < device_config::REMAP_CONTROL_COUNT {
                                                ui.label(remap_control_label(index, language));
                                                let target =
                                                    &mut self.selected_button_mapping.targets[index];
                                                eframe::egui::ComboBox::from_id_salt(format!(
                                                    "button_mapping_{index}"
                                                ))
                                                .width(145.0)
                                                .selected_text(remap_control_label(
                                                    usize::from(*target),
                                                    language,
                                                ))
                                                .show_ui(ui, |ui| {
                                                    for candidate in
                                                        0..device_config::REMAP_CONTROL_COUNT
                                                    {
                                                        ui.selectable_value(
                                                            target,
                                                            candidate as u8,
                                                            remap_control_label(candidate, language),
                                                        );
                                                    }
                                                });
                                            } else {
                                                ui.label("");
                                                ui.label("");
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            let changed = self
                                .device_button_mapping
                                .as_ref()
                                .is_some_and(|current| current != &self.selected_button_mapping);
                            if ui
                                .add_enabled(
                                    mapping_enabled && changed,
                                    primary_button(language.tr("应用并保存", "Apply and save")),
                                )
                                .clicked()
                            {
                                apply_button_mapping = true;
                            }
                            if ui
                                .add_enabled(
                                    mapping_enabled
                                        && self
                                            .device_button_mapping
                                            .as_ref()
                                            .is_some_and(|mapping| !mapping.is_identity()),
                                    eframe::egui::Button::new(language.tr(
                                        "恢复默认一对一",
                                        "Restore one-to-one",
                                    )),
                                )
                                .clicked()
                            {
                                reset_button_mapping = true;
                            }
                            if ui
                                .add_enabled(
                                    !self.loading_button_mapping
                                        && !self.applying_button_mapping,
                                    eframe::egui::Button::new(language.tr(
                                        "重新读取",
                                        "Read again",
                                    )),
                                )
                                .clicked()
                            {
                                refresh_button_mapping = true;
                            }
                        });
                        if let Some(error) = &self.button_mapping_error {
                            ui.add_space(8.0);
                            notice(
                                ui,
                                NoticeTone::Warning,
                                language.tr(
                                    "19 控件映射不可用",
                                    "19-control mapping unavailable",
                                ),
                                error,
                            );
                        } else if self.device_button_mapping.is_none() {
                            ui.add_space(8.0);
                            notice(
                                ui,
                                NoticeTone::Info,
                                language.tr("请先读取设备", "Read the device first"),
                                language.tr(
                                    "需连接运行新版固件的 M61；旧固件不会被当作 15 键配置导入。",
                                    "Connect an M61 running the new firmware. Legacy firmware is not imported as a 15-button configuration.",
                                ),
                            );
                        }
                    });

                    ui.add_space(12.0);

                    surface_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading(language.tr("USB 轮询性能", "USB polling performance"));
                            ui.with_layout(
                                eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                                |ui| {
                                    if self.loading_polling_rate || self.applying_polling_rate {
                                        ui.spinner();
                                    }
                                    ui.monospace(
                                        self.device_polling_rate
                                            .map(device_config::PollingRate::label)
                                            .unwrap_or(language.tr("未读取", "Not read")),
                                    );
                                },
                            );
                        });
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "常用版首次启动默认使用实时档（约 750 Hz）。修改后工具会保留其余设备配置并重启 M61，让 Windows 重新读取 USB 端点间隔。",
                                "A fresh Standard profile defaults to Realtime (~750 Hz). Applying a change preserves all other settings and restarts M61 so Windows reloads the USB endpoint interval.",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(language.tr("目标档位", "Target mode"));
                            eframe::egui::ComboBox::from_id_salt("device_polling_rate")
                                .selected_text(self.selected_polling_rate.label())
                                .show_ui(ui, |ui| {
                                    for mode in device_config::PollingRate::ALL {
                                        ui.selectable_value(
                                            &mut self.selected_polling_rate,
                                            mode,
                                            mode.label(),
                                        );
                                    }
                                });
                            if ui
                                .add_enabled(
                                    self.device_polling_rate.is_some()
                                        && self.device_polling_rate
                                            != Some(self.selected_polling_rate)
                                        && !self.loading_polling_rate
                                        && !self.applying_polling_rate
                                        && self.busy.is_none()
                                        && !running
                                        && !self.guided_test.active
                                        && !self.device_test_audio_busy,
                                    primary_button(language.tr(
                                        "应用并重启",
                                        "Apply and restart",
                                    )),
                                )
                                .clicked()
                            {
                                apply_polling_rate = Some(self.selected_polling_rate);
                            }
                            if ui
                                .add_enabled(
                                    !self.loading_polling_rate && !self.applying_polling_rate,
                                    eframe::egui::Button::new(language.tr(
                                        "重新读取",
                                        "Read again",
                                    )),
                                )
                                .clicked()
                            {
                                refresh_polling_rate = true;
                            }
                        });
                        if let Some(error) = &self.polling_rate_error {
                            ui.add_space(8.0);
                            notice(
                                ui,
                                NoticeTone::Warning,
                                language.tr("无法读取轮询档位", "Polling mode unavailable"),
                                error,
                            );
                        } else if self.device_polling_rate
                            == Some(device_config::PollingRate::Realtime)
                        {
                            ui.add_space(8.0);
                            notice(
                                ui,
                                NoticeTone::Success,
                                language.tr("实时档已启用", "Realtime mode enabled"),
                                language.tr(
                                    "USB 以 0.5 ms 服务间隔请求报告；实际输入上限仍受蓝牙报告到达速率影响，通常约 750 Hz。",
                                    "USB requests reports at a 0.5 ms service interval; actual input remains limited by Bluetooth report arrival, typically around 750 Hz.",
                                ),
                            );
                        }
                    });

                    ui.add_space(12.0);

                    surface_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading(language.tr("引导式设备诊断", "Guided device diagnostics"));
                            ui.with_layout(
                                eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
                                |ui| {
                                    ui.label(format!(
                                        "{}/{}",
                                        self.guided_test.current + 1,
                                        self.guided_test.phases.len()
                                    ));
                                },
                            );
                        });
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "不限时长；按键、方向键、摇杆、扳机、触摸和六轴输入会全程累计并自动判定。灯效、震动、扳机阻力和声音等主观输出由用户确认。",
                                "No time limit. Buttons, D-pad, sticks, triggers, touch and six-axis input are accumulated throughout the guide and assessed automatically. Subjective LED, rumble, trigger-resistance and sound outputs remain user-confirmed.",
                            ))
                            .color(COLOR_TEXT_MUTED),
                        );
                        if !self.guided_test.active {
                            let completed = self
                                .guided_test
                                .phases
                                .iter()
                                .filter(|phase| phase.result != guided_test::PhaseResult::Pending)
                                .count();
                            if completed > 0 {
                                let failed = self.guided_test.phases.iter().filter(|phase| {
                                    phase.result == guided_test::PhaseResult::NotEffective
                                }).count();
                                notice(
                                    ui,
                                    if failed == 0 && completed == self.guided_test.phases.len() {
                                        NoticeTone::Success
                                    } else {
                                        NoticeTone::Warning
                                    },
                                    language.tr("上次引导测试", "Previous guided test"),
                                    &match self.language {
                                        Language::ZhCn => format!(
                                            "已记录 {completed}/{} 项，其中 {failed} 项未生效；可直接导出报告或重新开始。",
                                            self.guided_test.phases.len()
                                        ),
                                        Language::En => format!(
                                            "Recorded {completed}/{} phases with {failed} not effective; export the report or restart.",
                                            self.guided_test.phases.len()
                                        ),
                                    },
                                );
                            }
                            if ui
                                .add_enabled(
                                    self.device_test_connected
                                        && !running
                                        && !self.loading_diagnostics,
                                    primary_button(if completed > 0 {
                                        language.tr("重新开始引导测试", "Restart guided test")
                                    } else {
                                        language.tr("开始引导测试", "Start guided test")
                                    }),
                                )
                                .clicked()
                            {
                                guide_start = true;
                            }
                        } else {
                            let automatic_phases = self
                                .guided_test
                                .phases
                                .iter()
                                .filter(|phase| phase.automatic)
                                .collect::<Vec<_>>();
                            let automatic_completed = automatic_phases
                                .iter()
                                .map(|phase| phase.completed_requirements())
                                .sum::<usize>();
                            let automatic_total = automatic_phases
                                .iter()
                                .map(|phase| phase.requirements.len())
                                .sum::<usize>();
                            let automatic_progress = if automatic_total == 0 {
                                0.0
                            } else {
                                automatic_completed as f32 / automatic_total as f32
                            };
                            ui.add(
                                eframe::egui::ProgressBar::new(automatic_progress)
                                    .desired_width(ui.available_width().min(620.0))
                                    .text(match self.language {
                                        Language::ZhCn => format!(
                                            "全部自动输入覆盖 {automatic_completed}/{automatic_total}"
                                        ),
                                        Language::En => format!(
                                            "Overall automatic input coverage {automatic_completed}/{automatic_total}"
                                        ),
                                    }),
                            );
                            ui.horizontal_wrapped(|ui| {
                                for phase in automatic_phases {
                                    let complete = phase.coverage_complete();
                                    let percent = (phase.coverage_percent() * 100.0).round() as u32;
                                    ui.label(
                                        eframe::egui::RichText::new(format!(
                                            "{} {} {percent}%",
                                            if complete { "✓" } else { "○" },
                                            language.tr(phase.title_zh, phase.title_en)
                                        ))
                                        .color(if complete {
                                            COLOR_SUCCESS
                                        } else {
                                            COLOR_TEXT_MUTED
                                        })
                                        .background_color(if complete {
                                            COLOR_SUCCESS_SOFT
                                        } else {
                                            COLOR_SURFACE_RAISED
                                        }),
                                    );
                                }
                            });
                            let phase = self.guided_test.phase();
                            ui.separator();
                            ui.strong(language.tr(phase.title_zh, phase.title_en));
                            ui.label(language.tr(phase.instruction_zh, phase.instruction_en));
                            ui.monospace(format!("Target: {}", phase.target));
                            let automatic_phase = phase.automatic;
                            if automatic_phase {
                                let completed = phase.completed_requirements();
                                let total = phase.requirements.len();
                                ui.add(
                                    eframe::egui::ProgressBar::new(phase.coverage_percent())
                                        .desired_width(ui.available_width().min(520.0))
                                        .text(match self.language {
                                            Language::ZhCn => {
                                                format!("自动覆盖 {completed}/{total}")
                                            }
                                            Language::En => {
                                                format!("Automatic coverage {completed}/{total}")
                                            }
                                        }),
                                );
                                ui.horizontal_wrapped(|ui| {
                                    for (key, target) in &phase.requirements {
                                        let observed =
                                            phase.samples.get(key).copied().unwrap_or_default();
                                        let complete = observed >= *target;
                                        let (symbol, color, background) = if complete {
                                            ("✓", COLOR_SUCCESS, COLOR_SUCCESS_SOFT)
                                        } else {
                                            ("○", COLOR_TEXT_MUTED, COLOR_SURFACE_RAISED)
                                        };
                                        ui.label(
                                            eframe::egui::RichText::new(format!(
                                                "{symbol} {} ({observed}/{target})",
                                                guided_requirement_label(key, language)
                                            ))
                                            .color(color)
                                            .background_color(background),
                                        );
                                    }
                                });
                                let missing = phase.missing_requirements();
                                if !missing.is_empty() {
                                    ui.label(
                                        eframe::egui::RichText::new(match self.language {
                                            Language::ZhCn => format!(
                                                "仍缺少 {} 项；完成后会自动进入下一步。",
                                                missing.len()
                                            ),
                                            Language::En => format!(
                                                "{} item(s) remain; the guide advances automatically when complete.",
                                                missing.len()
                                            ),
                                        })
                                        .color(COLOR_TEXT_MUTED),
                                    );
                                }
                            } else if !phase.samples.is_empty() {
                                ui.label(
                                    phase
                                        .samples
                                        .iter()
                                        .map(|(name, count)| format!("{name}={count}"))
                                        .collect::<Vec<_>>()
                                        .join(" · "),
                                );
                            }
                            let output_phase = matches!(
                                phase.id,
                                "leds" | "rumble" | "triggers_output" | "sound" | "microphone" | "summary"
                            );
                            let signal_label = match phase.id {
                                "microphone" => language.tr(
                                    "开始录音：20 秒说话 + 5 秒静音",
                                    "Record: 20 s voice + 5 s silence",
                                ),
                                "summary" => language.tr(
                                    "停止全部输出并采集快照",
                                    "Reset outputs and capture snapshot",
                                ),
                                _ => language.tr(
                                    "生成当前测试信号",
                                    "Generate current test signal",
                                ),
                            };
                            if output_phase
                                && ui
                                    .add_enabled(
                                        self.device_test_connected
                                            && !self.device_test_audio_busy,
                                        primary_button(signal_label),
                                    )
                                    .clicked()
                            {
                                guide_signal = true;
                            }
                            ui.label(language.tr(
                                "备注（可选；未生效时建议填写）",
                                "Note (optional; recommended when not effective)",
                            ));
                            ui.text_edit_singleline(&mut self.guided_test.phase_mut().note);
                            let phase_busy = self.guided_test.phase().id == "microphone"
                                && self.device_test_audio_busy;
                            ui.horizontal_wrapped(|ui| {
                                if automatic_phase {
                                    if ui
                                        .add_enabled(
                                            !phase_busy,
                                            eframe::egui::Button::new(language.tr(
                                                "无法完成，记录缺失项并下一步",
                                                "Cannot complete; record missing items and continue",
                                            )),
                                        )
                                        .clicked()
                                    {
                                        guide_result =
                                            Some(guided_test::PhaseResult::NotEffective);
                                    }
                                } else {
                                    if ui.add_enabled(
                                        self.device_test_connected && !phase_busy,
                                        eframe::egui::Button::new(language.tr(
                                            "通过并下一项",
                                            "Pass and next",
                                        )),
                                    ).clicked() {
                                        guide_result = Some(guided_test::PhaseResult::Pass);
                                    }
                                    if ui.add_enabled(
                                        !phase_busy,
                                        eframe::egui::Button::new(language.tr(
                                            "未生效并下一项",
                                            "Not effective and next",
                                        )),
                                    ).clicked() {
                                        guide_result = Some(guided_test::PhaseResult::NotEffective);
                                    }
                                }
                                if ui.add_enabled(
                                    !phase_busy,
                                    eframe::egui::Button::new(language.tr("跳过", "Skip")),
                                ).clicked() {
                                    guide_result = Some(guided_test::PhaseResult::Skipped);
                                }
                                if ui.add_enabled(
                                    !phase_busy,
                                    eframe::egui::Button::new(language.tr(
                                        "重新测试本项",
                                        "Retest phase",
                                    )),
                                ).clicked() {
                                    guide_retest = true;
                                }
                                if ui
                                    .add_enabled(
                                        !phase_busy,
                                        eframe::egui::Button::new(language.tr(
                                            "结束引导",
                                            "End guide",
                                        )),
                                    )
                                    .clicked()
                                {
                                    guide_cancel = true;
                                }
                            });
                        }
                        ui.separator();
                        let guide_report_has_data = self.guided_test.active
                            || guided_test_has_data(&self.guided_test.phases);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_enabled(
                                    guide_report_has_data
                                        && !self.loading_diagnostics
                                        && !self.device_debug_final_snapshot_pending,
                                    primary_button(language.tr(
                                        "导出引导诊断测试报告 JSON",
                                        "Export guided diagnostic report JSON",
                                    )),
                                )
                                .clicked()
                            {
                                export_report = true;
                            }
                            ui.label(
                                eframe::egui::RichText::new(language.tr(
                                    "导出为统一报告；自动输入覆盖、人工输出确认、麦克风指标和已采集的 M61 快照都会保留，未完成项目会明确标记。",
                                    "Exports the unified report with automatic input coverage, user-confirmed outputs, microphone metrics and captured M61 snapshots; unfinished items are explicitly marked.",
                                ))
                                .color(COLOR_TEXT_MUTED)
                                .small(),
                            );
                        });
                    });

                    ui.add_space(12.0);

                    surface_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.strong(language.tr("自动压力测试", "Automated stress test"));
                            ui.separator();
                            ui.label(language.tr("时长", "Duration"));
                            for seconds in [300_u32, 900, 1800, 3600] {
                                ui.add_enabled_ui(!running, |ui| {
                                    ui.radio_value(
                                        &mut self.device_debug_duration_secs,
                                        seconds,
                                        format!(
                                            "{} {}",
                                            seconds / 60,
                                            language.tr("分钟", "min")
                                        ),
                                    );
                                });
                            }
                            if !running {
                                if ui
                                    .add_enabled(
                                        self.device_test_connected
                                            && !extreme_duration_blocked
                                            && !self.device_test_audio_busy
                                            && self.device_test_controller_tone.is_none(),
                                        primary_button(language.tr("开始压力测试", "Start stress test")),
                                    )
                                    .clicked()
                                {
                                    start_benchmark = true;
                                }
                            } else if ui
                                .add(
                                    eframe::egui::Button::new(language.tr("提前停止", "Stop early"))
                                        .fill(COLOR_WARNING_SOFT)
                                        .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_WARNING)),
                                )
                                .clicked()
                            {
                                stop_benchmark = true;
                            }
                        });
                        ui.add_enabled_ui(!running, |ui| {
                            ui.checkbox(
                                &mut self.device_debug_stress_enabled,
                                language.tr(
                                    "启用输出负载（循环灯效、限强度震动和自适应扳机）",
                                    "Enable output load (cycling lights, limited rumble and adaptive triggers)",
                                ),
                            );
                        });
                        ui.add_enabled_ui(!running && self.device_debug_stress_enabled, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(language.tr("输出频率", "Output rate"));
                                ui.radio_value(
                                    &mut self.device_debug_stress_rate_hz,
                                    20,
                                    language.tr("20 Hz 推荐", "20 Hz recommended"),
                                );
                                ui.radio_value(
                                    &mut self.device_debug_stress_rate_hz,
                                    50,
                                    language.tr("50 Hz 极限", "50 Hz extreme"),
                                );
                            });
                        });
                        ui.label(
                            eframe::egui::RichText::new(language.tr(
                                "执行器按 15 秒工作、5 秒完全释放循环；每 5 秒自动采集 M61 快照，出现新的 ≥20 ms 停顿时追加采集。",
                                "Actuators cycle through 15 seconds active and 5 seconds fully released; M61 snapshots run every 5 seconds and additionally on new ≥20 ms stalls.",
                            ))
                            .color(COLOR_TEXT_MUTED)
                            .small(),
                        );
                        if self.guided_test.active {
                            notice(
                                ui,
                                NoticeTone::Info,
                                language.tr(
                                    "可直接开始性能测试",
                                    "Performance test can start now",
                                ),
                                language.tr(
                                    "开始后会结束当前引导流程并保留已经取得的结果，不再因引导尚未结束而锁住性能采样。",
                                    "Starting ends the current guide while preserving collected results; an unfinished guide no longer blocks performance sampling.",
                                ),
                            );
                        }
                        if self.device_debug_stress_enabled {
                            notice(
                                ui,
                                if self.device_debug_stress_rate_hz == 50 {
                                    NoticeTone::Warning
                                } else {
                                    NoticeTone::Info
                                },
                                language.tr("硬件负载提示", "Hardware load notice"),
                                language.tr(
                                    "长测会增加手柄耗电、马达温升和扳机机械负担。20 Hz 适合常规稳定性测试；50 Hz 仅建议短时极限链路测试，并保持手柄通风。",
                                    "Long runs increase battery drain, motor temperature and trigger mechanical duty. Use 20 Hz for normal stability testing; reserve 50 Hz for shorter extreme transport tests and keep the controller ventilated.",
                                ),
                            );
                            if extreme_duration_blocked {
                                notice(
                                    ui,
                                    NoticeTone::Error,
                                    language.tr(
                                        "极限模式时长过长",
                                        "Extreme mode duration is too long",
                                    ),
                                    language.tr(
                                        "由于设备没有执行器温度反馈，50 Hz 模式最多允许选择 15 分钟；30–60 分钟请使用 20 Hz。",
                                        "Because actuator temperature is unavailable, 50 Hz is limited to 15 minutes; use 20 Hz for 30–60 minute runs.",
                                    ),
                                );
                            }
                        }
                        if running {
                            let progress =
                                (elapsed_secs / self.device_debug_duration_secs as f32).clamp(0.0, 1.0);
                            ui.add(
                                eframe::egui::ProgressBar::new(progress)
                                    .animate(true)
                                    .text(format!(
                                        "{} / {}",
                                        format_duration((elapsed_secs * 1000.0) as u32),
                                        format_duration(
                                            self.device_debug_duration_secs.saturating_mul(1000)
                                        )
                                    )),
                            );
                        }
                    });

                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        let enough_samples = metrics.sample_count >= 100;
                        let average = metrics.average_interval_ms.max(0.001);
                        let latest_runtime = self.device_debug_final.as_ref().or_else(|| {
                            self.device_debug_runtime_samples.last().or_else(|| {
                                self.runtime_diagnostics
                                    .iter()
                                    .find_map(|report| report.snapshot.as_ref())
                            })
                        });
                        let (firmware_fault, firmware_warning) = self
                            .device_debug_baseline
                            .as_ref()
                            .zip(latest_runtime)
                            .map_or((false, false), |(before, after)| {
                                let loss_delta = after
                                    .loss_pressure
                                    .wrapping_sub(before.loss_pressure);
                                let mic_delta = after
                                    .mic_underruns
                                    .wrapping_sub(before.mic_underruns)
                                    + after.mic_overruns.wrapping_sub(before.mic_overruns);
                                let heap_drop = before
                                    .heap_free_bytes
                                    .saturating_sub(after.heap_free_bytes);
                                (loss_delta > 0 || mic_delta > 0, heap_drop > 8 * 1024)
                            });
                        let severe = enough_samples
                            && (metrics.report_rate_hz < 100.0
                                || metrics.maximum_interval_ms > 50.0
                                || metrics.gaps_over_10ms * 200 > metrics.sample_count
                                || firmware_fault);
                        let warning = enough_samples
                            && !severe
                            && (metrics.p99_interval_ms > average * 2.5
                                || metrics.jitter_stddev_ms > average * 0.75
                                || metrics.gaps_over_10ms > 0
                                || firmware_warning);
                        let (tone, result_title, result_body) = if !enough_samples {
                            (
                                NoticeTone::Info,
                                language.tr("等待有效样本", "Waiting for samples"),
                                language.tr(
                                    "连接设备后启动自动压力测试，并完整运行所选时长；默认 5 分钟适合快速自检。",
                                    "Connect the device, start the automated stress test, and run the selected duration; the default 5 minutes is suitable for a quick check.",
                                ),
                            )
                        } else if severe {
                            (
                                NoticeTone::Error,
                                language.tr("发现明显性能异常", "Significant performance issue detected"),
                                language.tr(
                                    "报告频率过低、存在超过 50 ms 的停顿、长间隔占比过高，或固件丢失/麦克风错误计数增加。请检查后续快照、USB 和蓝牙环境。",
                                    "Report rate is too low, a stall exceeded 50 ms, long gaps are excessive, or firmware loss/microphone error counters increased. Inspect the snapshots and USB/Bluetooth conditions.",
                                ),
                            )
                        } else if warning {
                            (
                                NoticeTone::Warning,
                                language.tr("存在调度抖动", "Scheduling jitter detected"),
                                language.tr(
                                    "平均吞吐正常，但尾部间隔、抖动或空闲堆变化偏高。建议先排除 USB/蓝牙干扰并重复一次 5 分钟测试；需要长期稳定性验证时再选择更长时长。",
                                    "Average throughput is normal, but tail intervals, jitter, or free-heap change is elevated. First remove USB/Bluetooth interference and repeat the 5-minute test; use a longer duration only for extended stability validation.",
                                ),
                            )
                        } else {
                            (
                                NoticeTone::Success,
                                language.tr("HID 调度表现稳定", "HID scheduling is stable"),
                                language.tr(
                                    "当前样本中未发现明显的长停顿或异常抖动。",
                                    "No material long stalls or abnormal jitter were found in this sample.",
                                ),
                            )
                        };
                        notice(ui, tone, result_title, result_body);
                        ui.add_space(12.0);

                        surface_frame().show(ui, |ui| {
                            ui.strong(language.tr("HID 延迟与抖动", "HID interval and jitter"));
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                metric_card(
                                    ui,
                                    language.tr("报告频率", "Report rate"),
                                    format!("{:.1} Hz", metrics.report_rate_hz),
                                    language.tr("Windows 实测", "Measured by Windows"),
                                    COLOR_ACCENT_HOVER,
                                );
                                metric_card(
                                    ui,
                                    language.tr("平均间隔", "Average interval"),
                                    format!("{:.3} ms", metrics.average_interval_ms),
                                    language.tr("越低越快", "Lower is faster"),
                                    COLOR_TEXT_PRIMARY,
                                );
                                metric_card(
                                    ui,
                                    "P95 / P99",
                                    format!(
                                        "{:.3} / {:.3} ms",
                                        metrics.p95_interval_ms, metrics.p99_interval_ms
                                    ),
                                    language.tr("尾部调度间隔", "Tail scheduling interval"),
                                    if warning || severe { COLOR_WARNING } else { COLOR_SUCCESS },
                                );
                                metric_card(
                                    ui,
                                    language.tr("最大间隔", "Maximum interval"),
                                    format!("{:.3} ms", metrics.maximum_interval_ms),
                                    language.tr("最长一次停顿", "Longest observed stall"),
                                    if metrics.maximum_interval_ms > 20.0 {
                                        COLOR_ERROR
                                    } else {
                                        COLOR_TEXT_PRIMARY
                                    },
                                );
                                metric_card(
                                    ui,
                                    language.tr("标准差抖动", "Jitter stddev"),
                                    format!("{:.3} ms", metrics.jitter_stddev_ms),
                                    language.tr("间隔离散程度", "Interval dispersion"),
                                    if warning || severe { COLOR_WARNING } else { COLOR_SUCCESS },
                                );
                                metric_card(
                                    ui,
                                    language.tr("长间隔次数", "Long gaps"),
                                    format!(
                                        ">5 ms: {}  ·  >10 ms: {}",
                                        metrics.gaps_over_5ms, metrics.gaps_over_10ms
                                    ),
                                    format!("{} {}", metrics.sample_count, language.tr("个样本", "samples")).as_str(),
                                    if metrics.gaps_over_10ms > 0 { COLOR_WARNING } else { COLOR_SUCCESS },
                                );
                                metric_card(
                                    ui,
                                    language.tr("压力输出报告", "Stress output reports"),
                                    metrics.stress_output_reports.to_string(),
                                    format!(
                                        "{} Hz · {}",
                                        self.device_debug_stress_rate_hz,
                                        language.tr(
                                            "15 秒负载 / 5 秒释放",
                                            "15 s load / 5 s release"
                                        )
                                    )
                                    .as_str(),
                                    if self.device_debug_stress_enabled {
                                        COLOR_ACCENT_HOVER
                                    } else {
                                        COLOR_TEXT_MUTED
                                    },
                                );
                            });
                        });

                        ui.add_space(12.0);
                        surface_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(language.tr("固件运行快照", "Firmware runtime snapshot"));
                                if running {
                                    ui.label(format!(
                                        "{}: {}",
                                        language.tr("周期样本", "Periodic samples"),
                                        self.device_debug_runtime_samples.len()
                                    ));
                                }
                                if self.loading_diagnostics {
                                    ui.spinner();
                                }
                                if ui
                                    .add_enabled(
                                        !self.loading_diagnostics,
                                        eframe::egui::Button::new(language.tr(
                                            "立即采集 M61 快照",
                                            "Capture M61 snapshot now",
                                        )),
                                    )
                                    .clicked()
                                {
                                    capture_snapshot = true;
                                }
                            });
                            if let Some(snapshot) = self
                                .runtime_diagnostics
                                .iter()
                                .find_map(|report| report.snapshot.as_ref())
                            {
                                eframe::egui::Grid::new("debug_runtime_snapshot")
                                    .num_columns(4)
                                    .spacing([20.0, 8.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.label("USB complete");
                                        ui.monospace(snapshot.usb_in_completed.to_string());
                                        ui.label("BT input");
                                        ui.monospace(snapshot.bt_input_reports.to_string());
                                        ui.end_row();
                                        ui.label("BT output");
                                        ui.monospace(snapshot.bt_output_completed.to_string());
                                        ui.label("RSSI");
                                        ui.monospace(snapshot.bt_rssi_dbm.map_or_else(
                                            || "—".to_owned(),
                                            |value| format!("{value} dBm"),
                                        ));
                                        ui.end_row();
                                        ui.label(language.tr("空闲堆", "Free heap"));
                                        ui.monospace(if snapshot.memory_status_flags & 1 != 0 {
                                            format_bytes(snapshot.heap_free_bytes)
                                        } else {
                                            "—".to_owned()
                                        });
                                        ui.label(language.tr("丢失压力", "Loss pressure"));
                                        ui.monospace(snapshot.loss_pressure.to_string());
                                        ui.end_row();
                                        ui.label("Mic underrun / overrun");
                                        ui.monospace(format!(
                                            "{} / {}",
                                            snapshot.mic_underruns, snapshot.mic_overruns
                                        ));
                                        ui.label("Audio pairs");
                                        ui.monospace(snapshot.bt_audio_pairs_submitted.to_string());
                                        ui.end_row();
                                        ui.label("Mic endpoint / queue");
                                        ui.monospace(format!(
                                            "{} / {}",
                                            snapshot.mic_endpoint_write_errors,
                                            snapshot.mic_queue_overruns.unwrap_or(0)
                                        ));
                                        ui.label("Mic decode / timing");
                                        ui.monospace(format!(
                                            "{} / {} µs",
                                            snapshot.mic_decode_errors.unwrap_or(0),
                                            snapshot.mic_decode_p99_us.unwrap_or(0)
                                        ));
                                        ui.end_row();
                                    });

                                ui.add_space(10.0);
                                ui.separator();
                                ui.strong(language.tr(
                                    "M61 内部转发延迟",
                                    "M61 internal bridge latency",
                                ));
                                ui.label(
                                    eframe::egui::RichText::new(language.tr(
                                        "蓝牙 HID 输入回调进入 M61 → USB IN 提交 → USB 传输完成；统计最近一个固件诊断窗口。",
                                        "Bluetooth HID callback enters M61 → USB IN submitted → USB transfer completed; values cover the latest firmware diagnostic window.",
                                    ))
                                    .color(COLOR_TEXT_MUTED),
                                );
                                if let Some(samples) = snapshot.bridge_latency_samples {
                                    if samples == 0 {
                                        notice(
                                            ui,
                                            NoticeTone::Info,
                                            language.tr("等待手柄输入", "Waiting for controller input"),
                                            language.tr(
                                                "当前诊断窗口没有完成的转发样本。保持手柄连接并操作按键或摇杆后重新采集。",
                                                "No completed bridge samples were recorded in this window. Keep the controller connected, use a button or stick, then capture again.",
                                            ),
                                        );
                                    } else {
                                        ui.horizontal_wrapped(|ui| {
                                            metric_card(
                                                ui,
                                                language.tr("窗口样本", "Window samples"),
                                                samples.to_string(),
                                                language.tr("约 1 秒窗口", "Approximately 1-second window"),
                                                COLOR_ACCENT_HOVER,
                                            );
                                            metric_card(
                                                ui,
                                                language.tr("接收 → USB 提交", "Receive → USB submit"),
                                                format!(
                                                    "{} / {} µs",
                                                    snapshot.bridge_rx_to_submit_avg_us.unwrap_or(0),
                                                    snapshot.bridge_rx_to_submit_max_us.unwrap_or(0)
                                                ),
                                                language.tr("平均 / 最大", "Average / maximum"),
                                                COLOR_TEXT_PRIMARY,
                                            );
                                            metric_card(
                                                ui,
                                                language.tr("USB 提交 → 完成", "USB submit → complete"),
                                                format!(
                                                    "{} / {} µs",
                                                    snapshot.bridge_usb_transfer_avg_us.unwrap_or(0),
                                                    snapshot.bridge_usb_transfer_max_us.unwrap_or(0)
                                                ),
                                                language.tr("平均 / 最大", "Average / maximum"),
                                                COLOR_TEXT_PRIMARY,
                                            );
                                            metric_card(
                                                ui,
                                                language.tr("M61 内部总延迟", "Total inside M61"),
                                                format!(
                                                    "{} / {} µs",
                                                    snapshot.bridge_total_avg_us.unwrap_or(0),
                                                    snapshot.bridge_total_max_us.unwrap_or(0)
                                                ),
                                                language.tr("平均 / 最大", "Average / maximum"),
                                                COLOR_SUCCESS,
                                            );
                                            metric_card(
                                                ui,
                                                language.tr("内部尾延迟", "Internal tail latency"),
                                                format!(
                                                    "≤{} / ≤{} µs",
                                                    snapshot.bridge_total_p95_us.unwrap_or(0),
                                                    snapshot.bridge_total_p99_us.unwrap_or(0)
                                                ),
                                                "P95 / P99",
                                                COLOR_SUCCESS,
                                            );
                                        });
                                        if snapshot.bridge_timing_flags.unwrap_or(0) != 0 {
                                            notice(
                                                ui,
                                                NoticeTone::Warning,
                                                language.tr("计时值已饱和", "Timing value saturated"),
                                                language.tr(
                                                    "至少一个内部延迟超过 65535 µs；请结合最大停顿、丢失压力和 USB 状态排查。",
                                                    "At least one internal latency exceeded 65535 µs; inspect maximum stalls, loss pressure and USB state.",
                                                ),
                                            );
                                        }
                                    }
                                } else {
                                    notice(
                                        ui,
                                        NoticeTone::Warning,
                                        language.tr("当前固件不支持内部计时", "Firmware does not expose internal timing"),
                                        language.tr(
                                            "旧版六页诊断仍可读取，但需要刷入带第 7 页桥接延迟数据的新版固件才能显示该项。",
                                            "The legacy six-page snapshot remains readable, but firmware with the seventh bridge-latency page is required for this section.",
                                        ),
                                    );
                                }

                                ui.add_space(10.0);
                                ui.separator();
                                ui.strong(language.tr(
                                    "固件任务与队列余量",
                                    "Firmware task and queue headroom",
                                ));
                                if let Some(tasks) = &snapshot.scheduler_tasks {
                                    eframe::egui::Grid::new("firmware_scheduler_metrics")
                                        .num_columns(4)
                                        .spacing([18.0, 6.0])
                                        .striped(true)
                                        .show(ui, |ui| {
                                            ui.strong(language.tr("任务", "Task"));
                                            ui.strong(language.tr(
                                                "最小空闲栈",
                                                "Minimum free stack",
                                            ));
                                            ui.strong(language.tr("CPU", "CPU"));
                                            ui.strong(language.tr("优先级", "Priority"));
                                            ui.end_row();
                                            for task in tasks {
                                                ui.monospace(&task.name);
                                                ui.monospace(task.stack_min_free_words.map_or_else(
                                                    || "—".to_owned(),
                                                    |words| format!(
                                                        "{words} words / {} B",
                                                        u32::from(words) * 4
                                                    ),
                                                ));
                                                ui.monospace(task.cpu_permille.map_or_else(
                                                    || "—".to_owned(),
                                                    |value| format!("{:.1}%", value as f32 / 10.0),
                                                ));
                                                ui.monospace(task.priority.map_or_else(
                                                    || "—".to_owned(),
                                                    |value| value.to_string(),
                                                ));
                                                ui.end_row();
                                            }
                                        });
                                    ui.label(
                                        eframe::egui::RichText::new(format!(
                                            "{}: {} / {} · {}: {} / {}",
                                            language.tr("输入队列 当前/峰值", "Input queue current/peak"),
                                            snapshot.bridge_input_queue_depth.unwrap_or(0),
                                            snapshot
                                                .bridge_input_queue_high_watermark
                                                .unwrap_or(0),
                                            language.tr("输出队列 当前/峰值", "Output queue current/peak"),
                                            snapshot.bridge_output_queue_depth.unwrap_or(0),
                                            snapshot
                                                .bridge_output_queue_high_watermark
                                                .unwrap_or(0),
                                        ))
                                        .color(COLOR_TEXT_MUTED),
                                    );
                                    let health = diagnostics::assess_runtime_health(snapshot);
                                    let body = if health.findings.is_empty() {
                                        if health.cpu_sample_available {
                                            language.tr(
                                                "当前样本的栈、CPU、堆、队列和内部尾延迟均未触发保守阈值。",
                                                "Stack, CPU, heap, queue, and internal tail-latency headroom did not cross the conservative thresholds in this sample.",
                                            ).to_owned()
                                        } else {
                                            language.tr(
                                                "栈、堆和队列未触发阈值；CPU 统计需要等待下一次诊断快照。",
                                                "Stack, heap, and queues did not cross a threshold; CPU accounting needs the next diagnostic snapshot.",
                                            ).to_owned()
                                        }
                                    } else {
                                        health
                                            .findings
                                            .iter()
                                            .map(|finding| runtime_health_finding_text(language, finding))
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    };
                                    let (tone, title) = match health.status {
                                        diagnostics::RuntimeHealthStatus::Healthy => (
                                            NoticeTone::Success,
                                            language.tr("运行余量正常", "Runtime headroom healthy"),
                                        ),
                                        diagnostics::RuntimeHealthStatus::Warning => (
                                            NoticeTone::Warning,
                                            language.tr("运行余量需要关注", "Runtime headroom needs attention"),
                                        ),
                                        diagnostics::RuntimeHealthStatus::Critical => (
                                            NoticeTone::Error,
                                            language.tr("检测到运行余量风险", "Runtime headroom risk detected"),
                                        ),
                                        diagnostics::RuntimeHealthStatus::Unavailable => (
                                            NoticeTone::Info,
                                            language.tr("运行余量数据不足", "Runtime headroom unavailable"),
                                        ),
                                    };
                                    ui.add_space(8.0);
                                    notice(ui, tone, title, &body);
                                } else {
                                    ui.label(
                                        eframe::egui::RichText::new(language.tr(
                                            "当前固件尚未提供任务栈、CPU 与桥接队列水位。",
                                            "This firmware does not expose task stack, CPU, or bridge queue headroom.",
                                        ))
                                        .color(COLOR_TEXT_MUTED),
                                    );
                                }
                            } else if let Some(error) = &self.diagnostics_error {
                                notice(
                                    ui,
                                    NoticeTone::Error,
                                    language.tr("快照采集失败", "Snapshot capture failed"),
                                    error,
                                );
                            } else {
                                ui.label(
                                    eframe::egui::RichText::new(language.tr(
                                        "尚未采集运行快照。基准测试开始与结束时会自动采集。",
                                        "No runtime snapshot yet. Benchmarks capture one at start and finish.",
                                    ))
                                    .color(COLOR_TEXT_MUTED),
                                );
                            }

                            if let (Some(before), Some(after)) =
                                (&self.device_debug_baseline, &self.device_debug_final)
                            {
                                let duration_ms = after.monotonic_ms.wrapping_sub(before.monotonic_ms);
                                if duration_ms > 0 {
                                    let seconds = duration_ms as f64 / 1000.0;
                                    ui.separator();
                                    ui.strong(language.tr("基准区间吞吐", "Benchmark interval throughput"));
                                    ui.label(format!(
                                        "USB {:.1}/s  ·  BT input {:.1}/s  ·  BT output {:.1}/s  ·  loss Δ{}  ·  mic error Δ{}  ·  audio error Δ{}",
                                        after.usb_in_completed.wrapping_sub(before.usb_in_completed) as f64 / seconds,
                                        after.bt_input_reports.wrapping_sub(before.bt_input_reports) as f64 / seconds,
                                        after.bt_output_completed.wrapping_sub(before.bt_output_completed) as f64 / seconds,
                                        after.loss_pressure.wrapping_sub(before.loss_pressure),
                                        diagnostics::microphone_error_delta(before, after),
                                        diagnostics::audio_error_delta(before, after),
                                    ));
                                }
                            }
                        });

                        ui.add_space(12.0);
                        let has_guided_results =
                            guided_test_has_data(&self.guided_test.phases);
                        let has_runtime_snapshot = self
                            .runtime_diagnostics
                            .iter()
                            .any(|report| report.snapshot.is_some());
                        let report_has_data = metrics.sample_count > 0
                            || has_guided_results
                            || has_runtime_snapshot
                            || self.controller_analyzer.left.samples() > 0
                            || self.controller_analyzer.right.samples() > 0
                            || !self.calibration_events.is_empty()
                            || self.last_microphone_metrics.is_some();
                        let report_waiting = self.loading_diagnostics
                            || self.device_debug_final_snapshot_pending;
                        if report_waiting {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(language.tr(
                                    "正在完成运行快照，完成后即可导出完整报告。",
                                    "Finishing the runtime snapshot; the complete report can be exported afterwards.",
                                ));
                            });
                        }
                        if ui
                            .add_enabled(
                                report_has_data && !report_waiting,
                                primary_button(language.tr(
                                    "导出完整测试报告 JSON",
                                    "Export complete test report JSON",
                                )),
                            )
                            .clicked()
                        {
                            export_report = true;
                        }
                    });
                });
                });

            if start_benchmark {
                self.start_debug_benchmark();
            }
            if stop_benchmark {
                self.stop_debug_benchmark(false);
            }
            if capture_snapshot {
                self.capture_debug_snapshot();
            }
            if export_report {
                self.export_debug_report();
            }
            if let Some(profile) = ota_switch {
                self.start_profile_ota(profile);
            }
            if refresh_polling_rate {
                self.refresh_polling_rate();
            }
            if let Some(mode) = apply_polling_rate {
                self.reset_test_outputs();
                self.apply_polling_rate(mode);
            }
            if guide_start {
                self.ensure_device_session();
                self.reset_test_outputs();
                self.guided_test.start();
                self.capture_debug_snapshot();
            }
            if guide_cancel {
                self.reset_test_outputs();
                self.guided_test.active = false;
                self.guided_test.require_input_resync();
                self.device_test_status = self
                    .language
                    .tr(
                        "引导测试已结束，已保留当前结果。",
                        "The guided test ended; current results were preserved.",
                    )
                    .to_owned();
            }
            if guide_retest {
                self.reset_test_outputs();
                self.guided_test.retest();
            }
            if guide_signal {
                self.generate_guided_test_signal();
            }
            if let Some(result) = guide_result {
                let phase_id = self.guided_test.phase().id;
                self.reset_test_outputs();
                let automatically_completed = self.guided_test.mark_and_next(result);
                let summary_completed = self.guided_test.finish_summary();
                if !automatically_completed.is_empty() {
                    let names = automatically_completed
                        .iter()
                        .map(|(zh, en)| self.language.tr(zh, en))
                        .collect::<Vec<_>>()
                        .join("、");
                    self.device_test_status = match self.language {
                        Language::ZhCn => format!("已自动完成：{names}"),
                        Language::En => format!("Automatically completed: {names}"),
                    };
                }
                if result == guided_test::PhaseResult::NotEffective
                    || phase_id == "summary"
                    || summary_completed
                {
                    self.capture_debug_snapshot();
                }
                if !self.guided_test.active {
                    self.device_test_status = self
                        .language
                        .tr(
                            "引导测试已完成，可导出完整测试报告 JSON。",
                            "Guided diagnostics completed; the complete test report JSON can now be exported.",
                        )
                        .to_owned();
                }
            }
        }

        if self.show_diagnostics_window {
            let summary = self.diagnostic_summary_text();
            let mut rerun = false;
            let mut export = false;
            let mut close = false;
            eframe::egui::Window::new(language.tr(
                "DS5Dongle 一键诊断",
                "DS5Dongle diagnostics",
            ))
            .collapsible(false)
            .resizable(true)
            .default_size([760.0, 560.0])
            .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(language.tr(
                    "同时检查 Windows 串口环境，并读取 0xFD/CRC32 九页运行态快照（兼容旧六至八页固件）。诊断数据仅保存在本机。",
                    "Checks the Windows serial environment and captures the nine-page 0xFD/CRC32 runtime snapshot, with legacy six- through eight-page compatibility. Diagnostic data stays local.",
                ));
                ui.add_space(6.0);
                if self.loading_diagnostics {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(language.tr("正在诊断...", "Running diagnostics..."));
                    });
                }
                ui.group(|ui| {
                    ui.strong(language.tr("诊断结论", "Diagnostic summary"));
                    ui.add(
                        eframe::egui::Label::new(
                            eframe::egui::RichText::new(&summary).monospace(),
                        )
                        .selectable(true)
                        .wrap(),
                    );
                });

                eframe::egui::ScrollArea::vertical()
                    .max_height(350.0)
                    .show(ui, |ui| {
                        for (index, report) in self.runtime_diagnostics.iter().enumerate() {
                            ui.add_space(8.0);
                            ui.strong(format!(
                                "{} · {:04X}:{:04X} · {}",
                                report.product_name,
                                report.vendor_id,
                                report.product_id,
                                report.firmware_version.as_deref().unwrap_or(language.tr(
                                    "固件版本未知",
                                    "firmware unknown"
                                ))
                            ));
                            if let Some(info) = &report.controller_firmware {
                                ui.label(format!(
                                    "{} · {} · {}{}",
                                    info.update_version,
                                    info.main_firmware_version,
                                    info.hardware_version,
                                    info.hardware_model
                                        .as_deref()
                                        .map_or_else(String::new, |model| format!(" · {model}"))
                                ));
                            }
                            if let Some(snapshot) = &report.snapshot {
                                eframe::egui::Grid::new(format!("diagnostic_metrics_{index}"))
                                    .num_columns(4)
                                    .spacing([18.0, 6.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.label(language.tr("快照", "Snapshot"));
                                        ui.monospace(format!("#{}", snapshot.snapshot_seq));
                                        ui.label(language.tr("运行时间", "Uptime"));
                                        ui.monospace(format_duration(snapshot.uptime_ms));
                                        ui.end_row();
                                        ui.label(language.tr("空闲堆", "Free heap"));
                                        ui.monospace(format!(
                                            "{} / {}",
                                            format_bytes(snapshot.heap_free_bytes),
                                            format_bytes(snapshot.heap_min_free_bytes)
                                        ));
                                        ui.label("RSSI");
                                        ui.monospace(snapshot.bt_rssi_dbm.map_or_else(
                                            || "—".to_owned(),
                                            |value| format!("{value} dBm"),
                                        ));
                                        ui.end_row();
                                        ui.label(language.tr("USB 完成", "USB complete"));
                                        ui.monospace(snapshot.usb_in_completed.to_string());
                                        ui.label(language.tr("蓝牙输入", "BT input"));
                                        ui.monospace(snapshot.bt_input_reports.to_string());
                                        ui.end_row();
                                        ui.label(language.tr("丢失/压力", "Loss/pressure"));
                                        ui.colored_label(
                                            if snapshot.loss_pressure == 0 {
                                                COLOR_SUCCESS
                                            } else {
                                                COLOR_WARNING
                                            },
                                            snapshot.loss_pressure.to_string(),
                                        );
                                        ui.label(language.tr("麦克风欠载", "Mic underrun"));
                                        ui.colored_label(
                                            if snapshot.mic_underruns == 0 {
                                                COLOR_SUCCESS
                                            } else {
                                                COLOR_WARNING
                                            },
                                            snapshot.mic_underruns.to_string(),
                                        );
                                        ui.end_row();
                                        ui.label(language.tr("麦克风接收/解码", "Mic received/decoded"));
                                        ui.monospace(format!(
                                            "{} / {}",
                                            snapshot.mic_frames_received.unwrap_or(0),
                                            snapshot.mic_frames_decoded.unwrap_or(0)
                                        ));
                                        ui.label(language.tr("麦克风队列/解码错误", "Mic queue/decode errors"));
                                        ui.colored_label(
                                            if snapshot.mic_queue_overruns.unwrap_or(0) == 0
                                                && snapshot.mic_decode_errors.unwrap_or(0) == 0
                                            {
                                                COLOR_SUCCESS
                                            } else {
                                                COLOR_WARNING
                                            },
                                            format!(
                                                "{} / {}",
                                                snapshot.mic_queue_overruns.unwrap_or(0),
                                                snapshot.mic_decode_errors.unwrap_or(0)
                                            ),
                                        );
                                        ui.end_row();
                                        ui.label(language.tr("麦克风解码 P99/最大", "Mic decode P99/max"));
                                        ui.monospace(format!(
                                            "{} / {} µs",
                                            snapshot.mic_decode_p99_us.unwrap_or(0),
                                            snapshot.mic_decode_max_us.unwrap_or(0)
                                        ));
                                        ui.label(language.tr("端点写入错误", "Endpoint write errors"));
                                        ui.colored_label(
                                            if snapshot.mic_endpoint_write_errors == 0 {
                                                COLOR_SUCCESS
                                            } else {
                                                COLOR_WARNING
                                            },
                                            snapshot.mic_endpoint_write_errors.to_string(),
                                        );
                                        ui.end_row();
                                        ui.label("OTA");
                                        ui.monospace(format!(
                                            "{} / {}",
                                            snapshot.ota_state, snapshot.ota_error
                                        ));
                                        ui.label(language.tr("电量", "Battery"));
                                        ui.monospace(snapshot.battery_percent.map_or_else(
                                            || "—".to_owned(),
                                            |value| format!("{value}%"),
                                        ));
                                        ui.end_row();
                                    });
                            } else if report.build_profile.as_deref() == Some("standard")
                                && report.error.is_none()
                            {
                                notice(
                                    ui,
                                    NoticeTone::Info,
                                    language.tr("常用版已识别", "Standard identified"),
                                    language.tr(
                                        "该配置不生成运行快照；需要内部桥接延迟和计数器时，请安全切换到诊断版。",
                                        "This profile does not produce runtime snapshots. Switch safely to Diagnostic for internal bridge latency and counters.",
                                    ),
                                );
                            } else if let Some(error) = &report.error {
                                notice(
                                    ui,
                                    NoticeTone::Error,
                                    language.tr("0xFD 诊断读取失败", "0xFD diagnostic capture failed"),
                                    error,
                                );
                            }
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !busy && !self.loading_diagnostics,
                            eframe::egui::Button::new(language.tr("重新诊断", "Run again")),
                        )
                        .clicked()
                    {
                        rerun = true;
                    }
                    if ui
                        .add_enabled(
                            !self.loading_diagnostics
                                && (!self.runtime_diagnostics.is_empty()
                                    || !self.devices.is_empty()),
                            eframe::egui::Button::new(
                                language.tr("保存 JSON 诊断包", "Save JSON bundle"),
                            ),
                        )
                        .clicked()
                    {
                        export = true;
                    }
                    if ui
                        .button(language.tr("复制摘要", "Copy summary"))
                        .clicked()
                    {
                        ui.ctx().copy_text(summary.clone());
                    }
                    if ui.button(language.tr("关闭", "Close")).clicked() {
                        close = true;
                    }
                });
            });
            if rerun {
                self.start_diagnostics();
            }
            if export {
                self.export_diagnostics();
            }
            if close {
                self.show_diagnostics_window = false;
            }
        }

        if self.confirm_macro_clear {
            let mut confirm = false;
            let mut cancel = false;
            eframe::egui::Window::new(language.tr(
                "清除设备宏数据",
                "Erase device macro data",
            ))
            .collapsible(false)
            .resizable(false)
            .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                notice(
                    ui,
                    NoticeTone::Error,
                    language.tr("该操作不可撤销", "This action cannot be undone"),
                    language.tr(
                        "将停止正在运行或录制的宏，删除设备 Flash 中 macro_store_a 与 macro_store_b 两个槽，并清空当前 RAM 宏。19 键映射、固件和电脑中的 JSON 文件不会被删除。",
                        "This stops active or recording macros, deletes both macro_store_a and macro_store_b from device Flash, and clears the active RAM macro. The 19-control mapping, firmware, and JSON files on this PC are not deleted.",
                    ),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(language.tr("取消", "Cancel")).clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(language.tr(
                            "确认清除并回读校验",
                            "Erase and verify",
                        ))
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
            if cancel {
                self.confirm_macro_clear = false;
            }
            if confirm {
                self.confirm_macro_clear = false;
                self.clear_device_macros();
            }
        }

        if self.show_isp_dialog {
            eframe::egui::Window::new(language.tr("进入 UART ISP", "Enter download mode"))
                .collapsible(false)
                .resizable(false)
                .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(language.tr(
                        "请按以下顺序操作开发板：",
                        "Use this sequence on the board:",
                    ));
                    ui.label(language.tr("1. 按住 BOOT", "1. Hold BOOT"));
                    ui.label(
                        language.tr("2. 点按并松开 RESET/RST", "2. Press and release RESET/RST"),
                    );
                    ui.label(language.tr("3. 松开 BOOT", "3. Release BOOT"));
                    ui.add_space(8.0);
                    notice(
                        ui,
                        NoticeTone::Error,
                        language.tr("刷写期间请勿断电", "Do not interrupt flashing"),
                        language.tr(
                            "刷写开始后不要拔线，也不要按 Reset。",
                            "Do not disconnect the cable or press Reset after flashing starts.",
                        ),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(language.tr("取消", "Cancel")).clicked() {
                            self.show_isp_dialog = false;
                        }
                        if ui
                            .button(
                                language
                                    .tr("我已进入 ISP，开始刷写", "ISP is ready — start flashing"),
                            )
                            .clicked()
                        {
                            self.show_isp_dialog = false;
                            self.start_flash();
                        }
                    });
                });
        }

        if self.show_driver_dialog {
            eframe::egui::Window::new(language.tr("安装 CH340 驱动", "Install CH340 driver"))
                .collapsible(false)
                .resizable(false)
                .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(language.tr(
                        "将从 wch-ic.com 通过 HTTPS 下载官方 CH341SER 安装器。",
                        "The official CH341SER installer will be downloaded over HTTPS from wch-ic.com.",
                    ));
                    ui.label(language.tr(
                        "工具会先验证 Authenticode 签名者，再弹出 Windows UAC。",
                        "The Authenticode signer is verified before Windows UAC is requested.",
                    ));
                    ui.horizontal(|ui| {
                        if ui.button(language.tr("取消", "Cancel")).clicked() {
                            self.show_driver_dialog = false;
                        }
                        if ui
                            .button(language.tr(
                                "下载并安装官方驱动",
                                "Download and install official driver",
                            ))
                            .clicked()
                        {
                            self.show_driver_dialog = false;
                            self.start_driver_install();
                        }
                    });
                });
        }

        if self.retry.is_some() {
            eframe::egui::Window::new(language.tr("刷写失败", "Flashing failed"))
                .collapsible(false)
                .resizable(false)
                .anchor(eframe::egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(language.tr(
                        "请再次确认开发板处于 BOOT+RESET 的 UART ISP 模式。",
                        "Confirm that the board is still in BOOT+RESET UART ISP mode.",
                    ));
                    ui.label(language.tr(
                        "可以用更慢但更兼容的 115200 baud 重试。失败日志会保留在临时目录。",
                        "You can retry at the slower, more compatible 115200 baud. Failure logs remain in the temporary directory.",
                    ));
                    ui.horizontal(|ui| {
                        if ui
                            .button(language.tr("关闭，不重试", "Close without retry"))
                            .clicked()
                        {
                            self.retry = None;
                        }
                        if ui
                            .button(language.tr(
                                "115200 baud 重试",
                                "Retry at 115200 baud",
                            ))
                            .clicked()
                            && let Some(retry) = self.retry.take()
                        {
                            self.start_retry(retry);
                        }
                    });
                });
        }

        if busy || self.loading_devices || self.loading_firmware_devices || self.loading_releases {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

pub(crate) const COLOR_APP_BG: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(244, 247, 251);
pub(crate) const COLOR_HEADER: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(255, 255, 255);
pub(crate) const COLOR_SURFACE: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(255, 255, 255);
pub(crate) const COLOR_SURFACE_RAISED: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(236, 242, 249);
pub(crate) const COLOR_BORDER: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(198, 210, 224);
pub(crate) const COLOR_TEXT_PRIMARY: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(19, 35, 57);
pub(crate) const COLOR_TEXT_MUTED: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(73, 91, 113);
pub(crate) const COLOR_ACCENT: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(0, 103, 184);
pub(crate) const COLOR_ACCENT_HOVER: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(0, 82, 148);
pub(crate) const COLOR_ACCENT_SOFT: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(222, 239, 253);
pub(crate) const COLOR_SUCCESS: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(20, 112, 68);
pub(crate) const COLOR_SUCCESS_SOFT: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(226, 246, 235);
pub(crate) const COLOR_WARNING: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(148, 91, 0);
pub(crate) const COLOR_WARNING_SOFT: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(255, 244, 214);
pub(crate) const COLOR_ERROR: eframe::egui::Color32 = eframe::egui::Color32::from_rgb(178, 32, 47);
pub(crate) const COLOR_ERROR_SOFT: eframe::egui::Color32 =
    eframe::egui::Color32::from_rgb(253, 230, 233);

#[derive(Clone, Copy)]
pub(crate) enum NoticeTone {
    Info,
    Success,
    Warning,
    Error,
}

pub(crate) fn configure_visual_style(ctx: &eframe::egui::Context) {
    let mut visuals = eframe::egui::Visuals::light();
    visuals.override_text_color = Some(COLOR_TEXT_PRIMARY);
    visuals.panel_fill = COLOR_APP_BG;
    visuals.window_fill = COLOR_SURFACE;
    visuals.extreme_bg_color = eframe::egui::Color32::WHITE;
    visuals.faint_bg_color = COLOR_SURFACE_RAISED;
    visuals.warn_fg_color = COLOR_WARNING;
    visuals.error_fg_color = COLOR_ERROR;
    visuals.hyperlink_color = COLOR_ACCENT_HOVER;
    visuals.selection.bg_fill = COLOR_ACCENT_SOFT;
    visuals.selection.stroke = eframe::egui::Stroke::new(1.5_f32, COLOR_ACCENT_HOVER);
    // Keep disabled controls visibly disabled without washing their labels into
    // the white background. The border/fill already communicates availability.
    visuals.disabled_alpha = 0.82;
    visuals.widgets.noninteractive.bg_fill = COLOR_SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = COLOR_SURFACE;
    visuals.widgets.noninteractive.bg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER);
    visuals.widgets.noninteractive.fg_stroke =
        eframe::egui::Stroke::new(1.0_f32, COLOR_TEXT_PRIMARY);
    visuals.widgets.noninteractive.corner_radius = 8.0.into();
    visuals.widgets.inactive.bg_fill = COLOR_SURFACE_RAISED;
    visuals.widgets.inactive.weak_bg_fill = COLOR_SURFACE_RAISED;
    visuals.widgets.inactive.bg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER);
    visuals.widgets.inactive.fg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_TEXT_PRIMARY);
    visuals.widgets.inactive.corner_radius = 8.0.into();
    visuals.widgets.hovered.bg_fill = eframe::egui::Color32::from_rgb(225, 237, 250);
    visuals.widgets.hovered.weak_bg_fill = eframe::egui::Color32::from_rgb(225, 237, 250);
    visuals.widgets.hovered.bg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_ACCENT_HOVER);
    visuals.widgets.hovered.fg_stroke = eframe::egui::Stroke::new(1.5_f32, COLOR_TEXT_PRIMARY);
    visuals.widgets.hovered.corner_radius = 8.0.into();
    visuals.widgets.active.bg_fill = COLOR_ACCENT_SOFT;
    visuals.widgets.active.weak_bg_fill = COLOR_ACCENT_SOFT;
    visuals.widgets.active.bg_stroke = eframe::egui::Stroke::new(1.5_f32, COLOR_ACCENT);
    visuals.widgets.active.fg_stroke = eframe::egui::Stroke::new(1.5_f32, COLOR_TEXT_PRIMARY);
    visuals.widgets.active.corner_radius = 8.0.into();
    visuals.widgets.open.fg_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_TEXT_PRIMARY);
    visuals.widgets.open.corner_radius = 8.0.into();
    visuals.window_stroke = eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = [10.0, 10.0].into();
        style.spacing.button_padding = [12.0, 7.0].into();
        style.spacing.interact_size.y = 34.0;
        style.spacing.indent = 18.0;
        style.visuals.menu_corner_radius = 10.0.into();
        style.text_styles.insert(
            eframe::egui::TextStyle::Heading,
            eframe::egui::FontId::proportional(21.0),
        );
        style.text_styles.insert(
            eframe::egui::TextStyle::Body,
            eframe::egui::FontId::proportional(15.0),
        );
        style.text_styles.insert(
            eframe::egui::TextStyle::Button,
            eframe::egui::FontId::proportional(14.0),
        );
    });
}

pub(crate) fn surface_frame() -> eframe::egui::Frame {
    eframe::egui::Frame::new()
        .fill(COLOR_SURFACE)
        .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER))
        .corner_radius(12.0)
        .inner_margin(16)
}

pub(crate) fn raised_frame() -> eframe::egui::Frame {
    eframe::egui::Frame::new()
        .fill(COLOR_SURFACE_RAISED)
        .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER))
        .corner_radius(10.0)
        .inner_margin(14)
}

pub(crate) fn page_heading(
    ui: &mut eframe::egui::Ui,
    eyebrow: &str,
    title: &str,
    description: &str,
) {
    ui.label(
        eframe::egui::RichText::new(eyebrow.to_uppercase())
            .color(COLOR_ACCENT_HOVER)
            .size(11.0)
            .strong(),
    );
    ui.label(
        eframe::egui::RichText::new(title)
            .color(COLOR_TEXT_PRIMARY)
            .size(24.0)
            .strong(),
    );
    ui.label(
        eframe::egui::RichText::new(description)
            .color(COLOR_TEXT_MUTED)
            .size(14.0),
    );
}

pub(crate) fn section_heading(ui: &mut eframe::egui::Ui, title: &str, description: &str) {
    ui.label(
        eframe::egui::RichText::new(title)
            .color(COLOR_TEXT_PRIMARY)
            .size(17.0)
            .strong(),
    );
    if !description.is_empty() {
        ui.label(
            eframe::egui::RichText::new(description)
                .color(COLOR_TEXT_MUTED)
                .small(),
        );
    }
}

pub(crate) fn status_badge(
    ui: &mut eframe::egui::Ui,
    text: &str,
    color: eframe::egui::Color32,
    fill: eframe::egui::Color32,
) {
    eframe::egui::Frame::new()
        .fill(fill)
        .stroke(eframe::egui::Stroke::new(1.0_f32, color))
        .corner_radius(20.0)
        .inner_margin(eframe::egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.label(
                eframe::egui::RichText::new(text)
                    .color(color)
                    .small()
                    .strong(),
            );
        });
}

pub(crate) fn notice(ui: &mut eframe::egui::Ui, tone: NoticeTone, title: &str, body: &str) {
    let (icon, color, fill) = match tone {
        NoticeTone::Info => ("●", COLOR_ACCENT_HOVER, COLOR_ACCENT_SOFT),
        NoticeTone::Success => ("✓", COLOR_SUCCESS, COLOR_SUCCESS_SOFT),
        NoticeTone::Warning => ("!", COLOR_WARNING, COLOR_WARNING_SOFT),
        NoticeTone::Error => ("×", COLOR_ERROR, COLOR_ERROR_SOFT),
    };
    eframe::egui::Frame::new()
        .fill(fill)
        .stroke(eframe::egui::Stroke::new(1.0_f32, color))
        .corner_radius(9.0)
        .inner_margin(eframe::egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    eframe::egui::RichText::new(icon)
                        .color(color)
                        .size(18.0)
                        .strong(),
                );
                ui.vertical(|ui| {
                    ui.label(eframe::egui::RichText::new(title).color(color).strong());
                    if !body.is_empty() {
                        ui.label(eframe::egui::RichText::new(body).color(COLOR_TEXT_PRIMARY));
                    }
                });
            });
        });
}

pub(crate) fn primary_button(text: &str) -> eframe::egui::Button<'_> {
    eframe::egui::Button::new(
        eframe::egui::RichText::new(text)
            .color(COLOR_TEXT_PRIMARY)
            .strong(),
    )
    .fill(COLOR_ACCENT_SOFT)
    .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_ACCENT))
}

pub(crate) fn secondary_button(text: &str) -> eframe::egui::Button<'_> {
    eframe::egui::Button::new(eframe::egui::RichText::new(text).color(COLOR_TEXT_PRIMARY))
        .fill(COLOR_SURFACE)
        .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER))
}

pub(crate) fn danger_button(text: &str) -> eframe::egui::Button<'_> {
    eframe::egui::Button::new(
        eframe::egui::RichText::new(text)
            .color(COLOR_ERROR)
            .strong(),
    )
    .fill(COLOR_ERROR_SOFT)
    .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_ERROR))
}

pub(crate) fn tab_button(text: &str, selected: bool, min_width: f32) -> eframe::egui::Button<'_> {
    eframe::egui::Button::new(
        eframe::egui::RichText::new(text)
            .color(if selected {
                COLOR_ACCENT_HOVER
            } else {
                COLOR_TEXT_MUTED
            })
            .strong(),
    )
    .fill(if selected {
        COLOR_ACCENT_SOFT
    } else {
        COLOR_SURFACE
    })
    .stroke(eframe::egui::Stroke::new(
        if selected { 1.5_f32 } else { 1.0_f32 },
        if selected { COLOR_ACCENT } else { COLOR_BORDER },
    ))
    .min_size([min_width, 40.0].into())
}

pub(crate) fn toggle_chip(text: &str, selected: bool) -> eframe::egui::Button<'_> {
    eframe::egui::Button::new(
        eframe::egui::RichText::new(text)
            .color(if selected {
                COLOR_ACCENT_HOVER
            } else {
                COLOR_TEXT_PRIMARY
            })
            .small()
            .strong(),
    )
    .fill(if selected {
        COLOR_ACCENT_SOFT
    } else {
        COLOR_SURFACE
    })
    .stroke(eframe::egui::Stroke::new(
        1.0_f32,
        if selected { COLOR_ACCENT } else { COLOR_BORDER },
    ))
    .min_size([0.0, 28.0].into())
}

pub(crate) fn metric_card(
    ui: &mut eframe::egui::Ui,
    label: &str,
    value: impl Into<String>,
    detail: &str,
    color: eframe::egui::Color32,
) {
    eframe::egui::Frame::new()
        .fill(COLOR_SURFACE_RAISED)
        .stroke(eframe::egui::Stroke::new(1.0_f32, COLOR_BORDER))
        .corner_radius(9.0)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_min_width(185.0);
            ui.label(eframe::egui::RichText::new(label).color(COLOR_TEXT_MUTED));
            ui.label(
                eframe::egui::RichText::new(value.into())
                    .color(color)
                    .size(22.0)
                    .strong()
                    .monospace(),
            );
            ui.label(
                eframe::egui::RichText::new(detail)
                    .color(COLOR_TEXT_MUTED)
                    .small(),
            );
        });
}

pub(crate) fn status_tone(status: &str) -> NoticeTone {
    let lower = status.to_ascii_lowercase();
    if status.contains("失败")
        || status.contains("错误")
        || lower.contains("failed")
        || lower.contains("error")
    {
        NoticeTone::Error
    } else if status.contains("未检测")
        || status.contains("未连接")
        || status.contains("忽略")
        || status.contains("不可")
        || lower.contains("not detected")
        || lower.contains("not connected")
        || lower.contains("ignored")
        || lower.contains("unavailable")
    {
        NoticeTone::Warning
    } else if status.contains("完成")
        || status.contains("成功")
        || status.contains("已连接")
        || lower.contains("completed")
        || lower.contains("success")
        || lower.contains("connected")
    {
        NoticeTone::Success
    } else {
        NoticeTone::Info
    }
}

//! The cabinet's palette and faces (dark only; colour carries state and nothing else).

use std::sync::Arc;

use egui::{Color32, FontData, FontDefinitions, FontFamily, RichText};

pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub error: Color32,
}

/// Values copied from server/webui/app.css (dark only; colour carries state and nothing else).
pub fn palette() -> Palette {
    Palette {
        bg: Color32::from_rgb(0x07, 0x09, 0x0a),    // --void
        panel: Color32::from_rgb(0x0e, 0x14, 0x12), // --panel
        text: Color32::from_rgb(0xe6, 0xf0, 0xea),  // --ink
        muted: Color32::from_rgb(0x8f, 0xa3, 0x9a), // --muted
        ok: Color32::from_rgb(0x4a, 0xde, 0x80),    // --guardian
        warn: Color32::from_rgb(0xf5, 0xc4, 0x51),  // --amber
        error: Color32::from_rgb(0xff, 0x5c, 0x6c), // --danger
    }
}

pub fn font_definitions() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        f.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("Inter", include_bytes!("../../../../../mobile/assets/fonts/Inter-Regular.ttf"));
    add("Inter-SemiBold", include_bytes!("../../../../../mobile/assets/fonts/Inter-SemiBold.ttf"));
    add("SpaceGrotesk", include_bytes!("../../../../../mobile/assets/fonts/SpaceGrotesk-Medium.ttf"));
    add("JetBrainsMono", include_bytes!("../../../../../mobile/assets/fonts/JetBrainsMono-Regular.ttf"));
    f.families.entry(FontFamily::Proportional).or_default().insert(0, "Inter".to_owned());
    f.families.entry(FontFamily::Monospace).or_default().insert(0, "JetBrainsMono".to_owned());
    f.families.insert(FontFamily::Name("heading".into()), vec!["SpaceGrotesk".into(), "Inter".into()]);
    f
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    let p = palette();
    let mut v = egui::Visuals::dark();
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.panel;
    v.override_text_color = Some(p.text);
    ctx.set_visuals(v);
}

pub fn mono(t: impl Into<String>) -> RichText {
    RichText::new(t).family(FontFamily::Monospace)
}

pub fn heading(t: impl Into<String>) -> RichText {
    RichText::new(t).family(FontFamily::Name("heading".into())).size(20.0)
}

pub fn status_color(s: guardian_core::diagnose::Status) -> Color32 {
    use guardian_core::diagnose::Status;
    let p = palette();
    match s {
        Status::Ok => p.ok,
        Status::Warn => p.warn,
        Status::Error => p.error,
        Status::Unknown => p.muted,
    }
}

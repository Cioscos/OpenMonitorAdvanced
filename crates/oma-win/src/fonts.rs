//! System font families through DirectWrite, for the overlay editor's font picker.

use windows::core::Result;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteLocalizedStrings, DWRITE_FACTORY_TYPE_SHARED,
};

/// At most this many families are returned.
const MAX_FAMILIES: usize = 2048;
/// Longer names are dropped (the `TextStyle.font` limit, in UTF-8 bytes).
const MAX_NAME_BYTES: usize = 64;

/// The family name from `(locale, name)` pairs: the `en-us` one (any case), else the first.
pub(crate) fn pick_family_name(names: &[(String, String)]) -> Option<String> {
    names
        .iter()
        .find(|(locale, _)| locale.eq_ignore_ascii_case("en-us"))
        .or_else(|| names.first())
        .map(|(_, name)| name.clone())
}

/// Installed font families, sorted case-insensitively, without duplicates.
pub fn system_font_families() -> Result<Vec<String>> {
    // SAFETY: plain factory creation; the returned COM object is owned and released on drop.
    let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
    let mut collection = None;
    // SAFETY: `collection` is a valid out slot for the whole call.
    unsafe { factory.GetSystemFontCollection(&mut collection, false)? };
    let collection = collection.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
    // SAFETY: `collection` is a live COM object.
    let count = unsafe { collection.GetFontFamilyCount() };
    let mut names = Vec::new();
    for i in 0..count {
        // Skip a family that fails instead of failing the whole list.
        // SAFETY: `i < count` and `collection` is live.
        let Ok(family) = (unsafe { collection.GetFontFamily(i) }) else {
            continue;
        };
        // SAFETY: `family` is a live COM object.
        let Ok(strings) = (unsafe { family.GetFamilyNames() }) else {
            continue;
        };
        if let Some(name) = read_names(&strings).as_deref().and_then(pick_family_name) {
            names.push(name);
        }
    }
    Ok(normalize_families(names))
}

/// Drops empty names and names over [`MAX_NAME_BYTES`], sorts and de-duplicates
/// case-insensitively (Unicode lowercase), and caps the list at [`MAX_FAMILIES`].
pub(crate) fn normalize_families(mut names: Vec<String>) -> Vec<String> {
    names.retain(|n| !n.is_empty() && n.len() <= MAX_NAME_BYTES);
    names.sort_by_cached_key(|n| n.to_lowercase());
    names.dedup_by(|a, b| a.to_lowercase() == b.to_lowercase());
    names.truncate(MAX_FAMILIES);
    names
}

/// All `(locale, name)` pairs of a localized string set; `None` if it cannot be read.
fn read_names(strings: &IDWriteLocalizedStrings) -> Option<Vec<(String, String)>> {
    // SAFETY: `strings` is a live COM object.
    let n = unsafe { strings.GetCount() };
    let mut pairs = Vec::with_capacity(n as usize);
    for i in 0..n {
        // SAFETY: `i < n`; the buffer is sized length + 1 for the NUL and live for the call.
        let locale_len = unsafe { strings.GetLocaleNameLength(i) }.ok()? as usize;
        let mut locale = vec![0u16; locale_len + 1];
        // SAFETY: `i < n`; `locale` holds `locale_len + 1` units, as GetLocaleName requires.
        unsafe { strings.GetLocaleName(i, &mut locale) }.ok()?;
        // SAFETY: `i < n` and `strings` is live.
        let name_len = unsafe { strings.GetStringLength(i) }.ok()? as usize;
        let mut name = vec![0u16; name_len + 1];
        // SAFETY: `i < n`; `name` holds `name_len + 1` units, as GetString requires.
        unsafe { strings.GetString(i, &mut name) }.ok()?;
        pairs.push((
            String::from_utf16_lossy(&locale[..locale_len]),
            String::from_utf16_lossy(&name[..name_len]),
        ));
    }
    Some(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(l: &str, n: &str) -> (String, String) {
        (l.into(), n.into())
    }

    #[test]
    fn pick_family_name_prefers_en_us() {
        let names = [p("it-it", "Corsivo"), p("EN-US", "Italic")];
        assert_eq!(pick_family_name(&names).as_deref(), Some("Italic"));
    }

    #[test]
    fn pick_family_name_falls_back_to_the_first() {
        let names = [p("it-it", "Corsivo"), p("de-de", "Kursiv")];
        assert_eq!(pick_family_name(&names).as_deref(), Some("Corsivo"));
        assert_eq!(pick_family_name(&[]), None);
    }

    #[test]
    fn normalize_families_filters_sorts_dedups_and_caps() {
        let long = "x".repeat(65);
        let ok64 = "y".repeat(64);
        let out = normalize_families(
            ["b", "", &long, "A", "a", "Ärger", "ärger", &ok64]
                .map(String::from)
                .to_vec(),
        );
        assert_eq!(out, ["A", "b", &ok64, "Ärger"]);
        let many: Vec<String> = (0..3000).map(|i| format!("f{i:04}")).collect();
        assert_eq!(normalize_families(many).len(), MAX_FAMILIES);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn system_fonts_include_segoe_ui() {
        let fonts = system_font_families().expect("fonts");
        assert!(fonts.iter().any(|f| f == "Segoe UI"), "{fonts:?}");
        assert!(fonts.len() <= MAX_FAMILIES);
        let mut sorted = fonts.clone();
        sorted.sort_by_cached_key(|n| n.to_lowercase());
        assert_eq!(fonts, sorted);
    }
}

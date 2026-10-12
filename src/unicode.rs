//! Unicode decomposition through Win32. The unicode-normalization tables cost
//! about 110 KiB of read-only data; NormalizeString and GetStringTypeW cost nothing.

use std::borrow::Cow;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_INSUFFICIENT_BUFFER};
use windows_sys::Win32::Globalization::{
    GetStringTypeW, NormalizationD, NormalizeString, C3_ALPHA, C3_NONSPACING, C3_VOWELMARK,
    CT_CTYPE3,
};

/// Canonical decomposition (NFD) with the combining marks dropped: "École" gives "Ecole".
/// ASCII text is returned as is without any allocation or Win32 call, and so is any
/// text Windows fails to normalize.
pub(crate) fn decomposed_without_marks(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    let mut source = Vec::with_capacity(text.len());
    let mut buffer = [0u16; 2];
    for character in text.chars() {
        // NormalizeString rejects a whole string holding a noncharacter (U+FFFE, U+FDD0…).
        // Those are never alphanumeric, so U+FFFD keeps the rest of the text foldable.
        let character = if is_noncharacter(character) {
            char::REPLACEMENT_CHARACTER
        } else {
            character
        };
        source.extend_from_slice(character.encode_utf16(&mut buffer));
    }
    let Some((units, types)) = decompose(source) else {
        return Cow::Borrowed(text);
    };
    let kept = units
        .iter()
        .zip(&types)
        .filter(|(_, flags)| !is_mark(**flags))
        .map(|(unit, _)| *unit);
    let mut result = String::with_capacity(text.len());
    result.extend(char::decode_utf16(kept).map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER)));
    Cow::Owned(result)
}

fn is_noncharacter(character: char) -> bool {
    let code = character as u32;
    (0xFDD0..=0xFDEF).contains(&code) || code & 0xFFFE == 0xFFFE
}

/// Nonspacing and vowel marks approximate Unicode category M. Windows also sets those
/// bits on a few letters (Ø, ø, Thai independent vowels), but letters carry C3_ALPHA
/// while combining marks never do. GetStringTypeW writes the u32 constants as u16.
fn is_mark(flags: u16) -> bool {
    flags & (C3_NONSPACING | C3_VOWELMARK) as u16 != 0 && flags & C3_ALPHA as u16 == 0
}

/// NFD code units of `source`, each paired with its CT_CTYPE3 flags. The source buffer
/// is reused for the flags, so a non-ASCII string costs two small allocations.
fn decompose(mut source: Vec<u16>) -> Option<(Vec<u16>, Vec<u16>)> {
    let length = i32::try_from(source.len()).ok()?;
    // SAFETY: a null destination only asks for a size estimate; the source stays valid.
    let mut capacity =
        unsafe { NormalizeString(NormalizationD, source.as_ptr(), length, null_mut(), 0) };
    let mut units = Vec::new();
    loop {
        if capacity <= 0 {
            return None;
        }
        units.resize(capacity as usize, 0);
        // SAFETY: the destination holds `capacity` code units for this synchronous call.
        let written = unsafe {
            NormalizeString(
                NormalizationD,
                source.as_ptr(),
                length,
                units.as_mut_ptr(),
                capacity,
            )
        };
        if written > 0 {
            units.truncate(written as usize);
            break;
        }
        // The documented retry: ERROR_INSUFFICIENT_BUFFER carries a larger estimate as
        // a negative count. Anything else keeps the original text.
        // SAFETY: reads the calling thread's last error code.
        if -written <= capacity || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
        capacity = -written;
    }
    let length = i32::try_from(units.len()).ok()?;
    source.clear();
    source.resize(units.len(), 0);
    // SAFETY: both buffers hold `length` code units for this synchronous call.
    if unsafe { GetStringTypeW(CT_CTYPE3, units.as_ptr(), length, source.as_mut_ptr()) } == 0 {
        return None;
    }
    Some((units, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

    fn reference(text: &str) -> String {
        text.nfd()
            .filter(|character| !is_combining_mark(*character))
            .collect()
    }

    #[test]
    fn ascii_is_borrowed_unchanged() {
        for text in ["", "Notepad++", "C# Dev Kit", "a\tb\r\n\u{7f}"] {
            assert!(matches!(
                decomposed_without_marks(text),
                Cow::Borrowed(same) if same == text
            ));
        }
    }

    #[test]
    fn matches_unicode_normalization_for_latin_greek_and_cyrillic() {
        let ranges = [
            '\u{a0}'..='\u{24f}',
            '\u{370}'..='\u{3ff}',
            '\u{400}'..='\u{4ff}',
            '\u{1e00}'..='\u{1eff}',
        ];
        for character in ranges.into_iter().flatten() {
            for text in [
                character.to_string(),
                format!("a{character}b"),
                format!("{character}{character}"),
            ] {
                assert_eq!(
                    decomposed_without_marks(&text),
                    reference(&text),
                    "U+{:04X} in {text:?}",
                    character as u32
                );
            }
        }
    }

    #[test]
    fn matches_unicode_normalization_for_fixture_titles() {
        for text in [
            "Déjà Vu",
            "Ångström",
            "Español",
            "Čeština",
            "Ελληνικά",
            "Tiếng Việt",
            "Ærøskøbing",
            "ﬁligrane",
            "한글",
            "हिन्दी",
            "Désinstaller Visual Studio Code",
            "Notepad++",
            "C# Dev Kit",
            "Zürich",
        ] {
            assert_eq!(decomposed_without_marks(text), reference(text), "{text:?}");
        }
        assert_eq!(decomposed_without_marks("ﬁligrane"), "ﬁligrane");
        assert_eq!(
            decomposed_without_marks("한글"),
            "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}"
        );
        assert_eq!(decomposed_without_marks("हिन्दी"), "हनद");
    }

    /// Divergences from unicode-normalization outside the Latin, Greek and Cyrillic blocks
    /// checked above. They come from the Windows NLS tables, not from this module, and are
    /// listed so that a change on either side shows up instead of passing unnoticed.
    #[test]
    fn documents_known_divergences_from_unicode_normalization() {
        let divergent = |text: &str, win32: &str| {
            assert_eq!(decomposed_without_marks(text), win32, "{text:?}");
            assert_ne!(reference(text), win32, "{text:?} no longer diverges");
        };
        // Spacing marks (Mc) that Windows flags neither as nonspacing nor as vowel marks
        // stay: Indic anusvara, visarga and length marks, including the length mark that
        // decomposing a two-part vowel sign or the Tamil letter AU produces.
        divergent("क\u{903}", "क\u{903}");
        divergent("ক\u{9cc}", "ক\u{9d7}");
        divergent("ஔ", "ஒ\u{bd7}");
        // Marks missing from the Windows tables stay: Combining Diacritical Marks Extended
        // and Supplement, Combining Half Marks, Cyrillic Extended-B.
        divergent("a\u{1ab0}", "a\u{1ab0}");
        divergent("a\u{1dfd}", "a\u{1dfd}");
        divergent("a\u{fe27}", "a\u{fe27}");
        divergent("а\u{a674}", "а\u{a674}");
        // Characters Unicode re-categorized after the Windows tables are still dropped: the
        // rub el hizb start, the Mongolian vowel separator and the New Tai Lue vowel signs.
        divergent("a\u{6de}b", "ab");
        divergent("a\u{180e}b", "ab");
        divergent("a\u{19b0}b", "ab");
        // GetStringTypeW classifies UTF-16 code units, so marks beyond the BMP stay even
        // when NormalizeString produced them.
        divergent("a\u{1d165}b", "a\u{1d165}b");
        divergent("a\u{e0100}b", "a\u{e0100}b");
        divergent("\u{1d15e}", "\u{1d157}\u{1d165}");
    }

    #[test]
    fn noncharacters_do_not_disable_folding_of_the_rest() {
        // NormalizeString rejects the whole input otherwise; the crate keeps them as is.
        for text in ["Café\u{fffe}", "\u{fdd0}Été", "Ångström\u{10ffff}"] {
            let folded = decomposed_without_marks(text);
            let expected: String = reference(text)
                .chars()
                .map(|c| if is_noncharacter(c) { '\u{fffd}' } else { c })
                .collect();
            assert_eq!(folded, expected, "{text:?}");
        }
    }
}

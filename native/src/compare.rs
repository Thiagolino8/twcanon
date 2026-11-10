// Port of Tailwind CSS 4.3.4 utils/compare.ts. See LICENSE.tailwind.
use icu_collator::{Collator, CollatorBorrowed, options::CollatorOptions};
use icu_locale_core::Locale;
use std::cmp::Ordering;

pub struct Collation {
    collator: CollatorBorrowed<'static>,
}

impl Collation {
    // Inglês usa a colação raiz; evita vincular os dados das outras localidades à CLI.
    pub(crate) fn english() -> Self {
        Self {
            collator: CollatorBorrowed::new_root(CollatorOptions::default()),
        }
    }

    pub fn new(locale: &str) -> Result<Self, String> {
        let locale: Locale = locale
            .parse()
            .map_err(|error| format!("Invalid locale {locale:?}: {error}"))?;
        let collator = Collator::try_new(locale.into(), CollatorOptions::default())
            .map_err(|error| error.to_string())?;
        Ok(Self { collator })
    }

    pub fn compare(&self, left: &str, right: &str) -> Ordering {
        self.collator.compare(left, right)
    }
}

pub fn natural(left: &str, right: &str) -> Ordering {
    if left.is_ascii() && right.is_ascii() {
        let (left, right) = (left.as_bytes(), right.as_bytes());
        let mut i = 0;
        while i < left.len().min(right.len()) {
            if left[i].is_ascii_digit() && right[i].is_ascii_digit() {
                let left_end = i + left[i..].iter().take_while(|c| c.is_ascii_digit()).count();
                let right_end = i + right[i..].iter().take_while(|c| c.is_ascii_digit()).count();
                let (a, b) = (&left[i..left_end], &right[i..right_end]);
                if a == b {
                    i = left_end;
                    continue;
                }
                let number = |digits| std::str::from_utf8(digits).unwrap().parse::<f64>().unwrap();
                let difference = number(a) - number(b);
                if difference < 0.0 {
                    return Ordering::Less;
                }
                if difference > 0.0 {
                    return Ordering::Greater;
                }
                return a.cmp(b);
            }
            match left[i].cmp(&right[i]) {
                Ordering::Equal => i += 1,
                result => return result,
            }
        }
        return left.len().cmp(&right.len());
    }
    let left: Vec<u16> = left.encode_utf16().collect();
    let right: Vec<u16> = right.encode_utf16().collect();
    let digit = |c: &u16| (48..=57).contains(c);
    for i in 0..left.len().min(right.len()) {
        if digit(&left[i]) && digit(&right[i]) {
            let left_end = i + left[i..].iter().take_while(|c| digit(c)).count();
            let right_end = i + right[i..].iter().take_while(|c| digit(c)).count();
            let a = &left[i..left_end];
            let b = &right[i..right_end];
            let number = |n: &[u16]| {
                n.iter()
                    .map(|c| char::from_u32(u32::from(*c)).unwrap())
                    .collect::<String>()
                    .parse::<f64>()
                    .unwrap()
            };
            let difference = number(a) - number(b);
            if difference < 0.0 {
                return Ordering::Less;
            }
            if difference > 0.0 {
                return Ordering::Greater;
            }
            match a.cmp(b) {
                Ordering::Equal => continue,
                result => return result,
            }
        }
        match left[i].cmp(&right[i]) {
            Ordering::Equal => {}
            result => return result,
        }
    }
    left.len().cmp(&right.len())
}

#[cfg(test)]
mod tests {
    use super::Collation;
    use serde::Deserialize;

    #[test]
    fn root_collation_matches_english() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Fixture {
            compare_cases: Vec<Case>,
        }
        #[derive(Deserialize)]
        struct Case {
            left: String,
            right: String,
        }

        let fixture: Fixture =
            serde_json::from_str(include_str!("../fixtures/compare.json")).unwrap();
        let english = Collation::english();
        for locale in ["en", "en-US"] {
            let reference = Collation::new(locale).unwrap();
            for case in &fixture.compare_cases {
                assert_eq!(
                    english.compare(&case.left, &case.right),
                    reference.compare(&case.left, &case.right),
                    "{locale}: {:?} vs {:?}",
                    case.left,
                    case.right,
                );
            }
            let values = [
                "a", "A", "á", "ä", "å", "æ", "ß", "İ", "ı", "ﬀ", "é", "e\u{301}", "α", "я", "ع",
                "א", "क", "中", "中文", "日", "あ", "ア", "한", "😀", "𐐀", "-", "_", "1", "10",
            ];
            for left in values {
                for right in values {
                    assert_eq!(
                        english.compare(left, right),
                        reference.compare(left, right),
                        "{locale}: {left:?} vs {right:?}",
                    );
                }
            }
        }
    }
}

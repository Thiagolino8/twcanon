// Port of Tailwind CSS 4.3.4 utils/compare.ts. See LICENSE.tailwind.
use icu_collator::{Collator, CollatorBorrowed, options::CollatorOptions};
use icu_locale_core::Locale;
use std::cmp::Ordering;

pub struct Collation {
    collator: CollatorBorrowed<'static>,
}

impl Collation {
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

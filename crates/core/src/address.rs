//! Phone numbers and email addresses as keys.
//!
//! The same person shows up as `+15550101234` in a handle, `(555)
//! 010-1234` in a contact card and `555.010.1234` in an SMS chat. Every
//! address is normalized before it is stored or looked up: phone numbers
//! to E.164 (in the user's region when they have no country code), emails
//! to lowercase. Anything else (short codes, business IDs) is kept as is,
//! minus whitespace.

use phonenumber::{Mode, country};

/// The region for numbers written without a country code: the `country`
/// of a handle, else the user's locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region(Option<country::Id>);

impl Region {
    pub fn new(code: &str) -> Self {
        Self(code.trim().to_ascii_uppercase().parse().ok())
    }

    /// From `LC_ALL`, `LC_MESSAGES` or `LANG` (`en_US.UTF-8` → US).
    pub fn from_locale() -> Self {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|k| std::env::var(k).ok())
            .find(|v| !v.is_empty())
            .and_then(|v| {
                let region = v.split(['.', '@']).next()?.split('_').nth(1)?.to_owned();
                Some(Self::new(&region))
            })
            .filter(|r| r.0.is_some())
            .unwrap_or(Self::new("US"))
    }

    pub fn code(&self) -> Option<&str> {
        self.0.as_ref().map(|id| id.as_ref())
    }
}

pub fn is_email(address: &str) -> bool {
    address.contains('@')
}

/// The lookup key for an address.
pub fn normalize(address: &str, region: Region) -> String {
    let address = address.trim();
    if is_email(address) {
        return address.to_lowercase();
    }
    let digits = address.chars().filter(char::is_ascii_digit).count();
    let only_phone_chars = address
        .chars()
        .all(|c| c.is_ascii_digit() || " +-().\u{a0}/".contains(c));
    // Short codes (5 or 6 digits) and alphanumeric senders stay as they are.
    if !only_phone_chars || digits < 7 {
        return address.split_whitespace().collect();
    }
    // Numbers the metadata calls invalid (new ranges, the 555 test range)
    // still parse into country code and national number.
    match phonenumber::parse(region.0, address) {
        Ok(number) => number.format().mode(Mode::E164).to_string(),
        Err(_) => {
            // Keep the digits so two spellings still meet.
            let plus = if address.starts_with('+') { "+" } else { "" };
            format!(
                "{plus}{}",
                address
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect::<String>()
            )
        }
    }
}

/// How an address without a contact is shown: national format for numbers
/// in the user's region, international otherwise.
pub fn display(address: &str, region: Region) -> String {
    let address = address.trim();
    if is_email(address) {
        return address.to_owned();
    }
    match phonenumber::parse(region.0, address) {
        Ok(number) => {
            let home = region
                .code()
                .and_then(|id| phonenumber::metadata::DATABASE.by_id(id))
                .map(|m| m.country_code());
            let mode = if Some(number.country().code()) == home {
                Mode::National
            } else {
                Mode::International
            };
            number.format().mode(mode).to_string()
        }
        _ => address.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_meet() {
        let us = Region::new("us");
        for spelling in [
            "+15550101234",
            "(555) 010-1234",
            "555.010.1234",
            "1 555 010 1234",
            "+1 (555) 010-1234",
        ] {
            assert_eq!(normalize(spelling, us), "+15550101234", "{spelling}");
        }
        assert_eq!(
            normalize(" Alex.Kim@Example.com ", us),
            "alex.kim@example.com"
        );
        assert_eq!(normalize("12345", us), "12345");
        assert_eq!(normalize("Amazon", us), "Amazon");
    }

    #[test]
    fn region_matters_without_a_country_code() {
        assert_eq!(
            normalize("020 7946 0958", Region::new("GB")),
            "+442079460958"
        );
        assert_eq!(
            normalize("+44 20 7946 0958", Region::new("US")),
            "+442079460958"
        );
        assert_eq!(
            normalize("07700 900123", Region::new("GB")),
            "+447700900123"
        );
    }

    #[test]
    fn display_formats() {
        let us = Region::new("US");
        assert_eq!(display("+15550101234", us), "(555) 010-1234");
        assert_eq!(display("+442079460958", us), "+44 20 7946 0958");
        assert_eq!(display("a@b.com", us), "a@b.com");
    }

    #[test]
    fn locale_region() {
        assert_eq!(Region::new("de").code(), Some("DE"));
        assert_eq!(Region::new("zz").code(), None);
    }
}

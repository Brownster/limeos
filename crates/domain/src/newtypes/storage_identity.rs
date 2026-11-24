//! Storage device ids and filesystem UUIDs.

/// The storage-device-id rule: 1 to 64 bytes of lowercase letters, digits,
/// `_` and `-`, beginning with a letter or digit. The `id` of a contract
/// device, a wait-plan device and a target.
pub fn storage_device_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
}

/// The filesystem-UUID rule for a UUID an operator or planner chooses (a
/// contract device, a wait plan, a protection drive): 1 to 128 bytes of ASCII
/// letters, digits, `.`, `_` and `-`, beginning with a letter or digit. Case
/// is kept and compared exactly (blkid reports ext4 UUIDs in lowercase, vfat
/// serials in uppercase).
pub fn filesystem_uuid(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && observed_filesystem_uuid(value)
}

/// The same alphabet and bound without the first-byte rule, for a UUID the
/// host reported (the inventory, read from blkid). Kept lenient on purpose:
/// it parses inventories stored in plans that predate any first-byte rule,
/// and an observed value only ever matches a contract value by equality, so
/// a UUID the contract rule refuses can never be selected.
pub fn observed_filesystem_uuid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}

#[cfg(test)]
mod tests;

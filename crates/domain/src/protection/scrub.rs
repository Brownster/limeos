//! Numeric manual scrub intent and source eligibility, without process authority.
use super::*;

/// Preserve omission and explicit zero; never inherit saved configuration defaults.
/// A registered runtime must bind its selected package's defaults before approval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidScrubRequest {
    pub percent: Option<u8>,
    pub age_days: Option<u16>,
}
impl SnapraidScrubRequest {
    pub fn validate(&self) -> StoragePlanningResult<()> {
        if self.percent.is_some_and(|percent| percent > 100) {
            return Err(vec![finding(
                "request.percent",
                StoragePlanningFindingCode::InvalidConfiguration,
            )]);
        }
        Ok(())
    }
}

/// Serializable review data, not a retained source owner or executable approval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapraidScrubPreview {
    pub configuration: SnapraidPreview,
    pub expected_sources: ProtectionSourceEvidence,
    pub request: SnapraidScrubRequest,
}

/// Check all original source roles, then require existing content/parity inputs.
/// Scrub writes content metadata; caller-owned locks/authority remain mandatory.
/// No diff, threshold override, schedule authorization or command execution occurs.
pub fn plan_snapraid_scrub(
    config: &SnapraidConfig,
    pools: &PoolsConfig,
    contract: &StorageContract,
    sources: &ProtectionSourceEvidence,
    request: SnapraidScrubRequest,
    now: i64,
) -> StoragePlanningResult<SnapraidScrubPreview> {
    use StoragePlanningFindingCode::*;
    request.validate()?;
    let preview = config.plan(pools, contract)?;
    if !config.enabled {
        return Err(vec![finding("enabled", InvalidConfiguration)]);
    }
    preview.check_sources(sources, now)?;
    for (index, (required, observed)) in preview.requirements.iter().zip(&sources.paths).enumerate()
    {
        if matches!(
            required.kind,
            ProtectionPathKind::Content | ProtectionPathKind::Parity
        ) && observed.state != ProtectionPathState::RegularFile
        {
            return Err(vec![finding(
                &format!("sources.paths[{index}]"),
                UnverifiedSource,
            )]);
        }
    }
    Ok(SnapraidScrubPreview {
        configuration: preview,
        expected_sources: sources.clone(),
        request,
    })
}

#[cfg(test)]
mod tests;

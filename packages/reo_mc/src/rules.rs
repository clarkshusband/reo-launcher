use interfrost::api::minecraft::{Os, OsRule, Rule, RuleAction};

use reo_common::os_ext::OsExt;

/// The last rule that applies decides as in the vanilla launcher
#[must_use]
pub fn validate_rules(rules: &[Rule], java_arch: &str, updated: bool) -> bool {
    let decided = rules
        .iter()
        .rev()
        .find(|rule| rule_applies(rule, java_arch, updated))
        .map(|rule| matches!(rule.action, RuleAction::Allow));

    // a list of disallows none of which applied leaves the entry in
    decided.unwrap_or_else(|| {
        rules
            .iter()
            .all(|rule| matches!(rule.action, RuleAction::Disallow))
    })
}

fn rule_applies(rule: &Rule, java_arch: &str, updated: bool) -> bool {
    match rule {
        Rule { os: Some(os), .. } => validate_os_rule(os, java_arch, updated),
        Rule {
            features: Some(features),
            ..
        } => {
            !features.is_demo_user.unwrap_or(true)
                || features.has_custom_resolution.unwrap_or(false)
                || !features.has_quick_plays_support.unwrap_or(true)
                || !features.is_quick_play_multiplayer.unwrap_or(true)
                || !features.is_quick_play_realms.unwrap_or(true)
                || !features.is_quick_play_singleplayer.unwrap_or(true)
        }
        // no condition so only the action matters
        _ => true,
    }
}

#[must_use]
pub fn validate_os_rule(rule: &OsRule, java_arch: &str, updated: bool) -> bool {
    let mut rule_match = true;

    if let Some(ref arch) = rule.arch {
        rule_match &= !matches!(arch.as_str(), "x86" | "arm");
    }

    if let Some(name) = &rule.name {
        // newer versions list every arch's natives under the plain os name
        // an arch-specific name still only matches that arch
        if updated && name.get_os() == *name {
            rule_match &= Os::native() == *name;
        } else {
            rule_match &= Os::native_arch(java_arch) == *name;
        }
    }

    rule_match
}

#[cfg(test)]
mod tests {
    use super::{Os, OsExt, OsRule, Rule, RuleAction, validate_os_rule, validate_rules};

    fn rule(action: RuleAction, os: Option<Os>) -> Rule {
        Rule {
            action,
            os: os.map(|name| OsRule {
                name: Some(name),
                version: None,
                arch: None,
            }),
            features: None,
        }
    }

    #[test]
    fn a_bare_disallow_keeps_the_entry_out() {
        assert!(!validate_rules(
            &[rule(RuleAction::Disallow, None)],
            "x86_64",
            true
        ));
        assert!(validate_rules(&[rule(RuleAction::Allow, None)], "x86_64", true));
    }

    #[test]
    fn a_later_rule_overrides_a_bare_disallow() {
        let host = Os::native_arch("x86_64");
        let rules = [
            rule(RuleAction::Disallow, None),
            rule(RuleAction::Allow, Some(host)),
        ];

        assert!(validate_rules(&rules, "x86_64", false));
    }

    #[test]
    fn an_arm_rule_never_matches_an_x86_64_runtime() {
        for name in [Os::OsxArm64, Os::WindowsArm64, Os::LinuxArm64, Os::LinuxArm32] {
            for updated in [true, false] {
                let os = OsRule {
                    name: Some(name.clone()),
                    version: None,
                    arch: None,
                };
                assert!(
                    !validate_os_rule(&os, "x86_64", updated),
                    "{name:?} matched an x86_64 runtime (updated: {updated})"
                );
            }
        }
    }
}

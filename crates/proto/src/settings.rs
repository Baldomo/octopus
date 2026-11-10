//! The settings an account's user carries to every device: whole values its member devices sign, of which the newest
//! of each setting stands.

use std::collections::BTreeMap;
use std::mem::{Discriminant, discriminant};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::account::{self, AccountId, MAX_CLOCK_AHEAD_MS, Signature};
use crate::device::{DeviceKey, PublicKey};
use crate::schema::{Shape, arg_shape};

const SET_LABEL: &[u8] = b"octopus account setting";

/// A harness's agent flags as the user typed them, shell words that split into the args each focus sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(try_from = "String")]
pub struct AgentFlags(String);

impl TryFrom<String> for AgentFlags {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        match shell_words::split(&text) {
            Ok(_) => Ok(Self(text.trim().to_string())),
            Err(_) => Err(format!("the quoting of {text} does not close")),
        }
    }
}

impl AgentFlags {
    pub fn args(&self) -> Vec<String> {
        shell_words::split(&self.0).expect("agent flags hold only text that splits")
    }
}

/// One setting's whole value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "setting", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub enum Setting {
    /// The sections by name in the order the user dragged them to.
    SectionOrder(Vec<String>),
    /// The collapsed sections by name.
    Collapsed(Vec<String>),
    /// The flags each harness's agents start with, by harness id.
    AgentFlags(BTreeMap<String, AgentFlags>),
}

/// The settings as they read: each the newest value set, else empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
pub struct Current {
    pub section_order: Vec<String>,
    pub collapsed: Vec<String>,
    pub agent_flags: BTreeMap<String, AgentFlags>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    /// When it was set, in milliseconds since 1970.
    pub at: i64,
    pub setting: Setting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedSet {
    pub set: Set,
    pub by: PublicKey,
    pub signature: Signature,
}

impl SignedSet {
    fn kind(&self) -> Discriminant<Setting> {
        discriminant(&self.set.setting)
    }

    /// Where it falls among values of its setting: by its time, then its device's key.
    fn order(&self) -> (i64, [u8; 32]) {
        (self.set.at, *self.by.as_bytes())
    }
}

/// An account's settings: the newest value of each setting each member device signed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub account: AccountId,
    pub sets: Vec<SignedSet>,
}

arg_shape!(Shape::Text => Settings);

impl Settings {
    pub fn new(account: AccountId) -> Self {
        Self { account, sets: Vec::new() }
    }

    /// Sets `setting` as `key`'s, later than every value of it these settings hold.
    pub fn set(&mut self, key: &DeviceKey, setting: Setting) {
        let kind = discriminant(&setting);
        let latest = self.sets.iter().filter(|held| held.kind() == kind).map(|held| held.set.at + 1).max();
        self.sign(key, crate::now().max(latest.unwrap_or(i64::MIN)), setting);
    }

    /// Sets `setting` as `key`'s, older than every value set on any device: a value the device kept before its
    /// account held settings, which yields to whatever the account already holds.
    pub fn seed(&mut self, key: &DeviceKey, setting: Setting) {
        self.sign(key, 0, setting);
    }

    fn sign(&mut self, key: &DeviceKey, at: i64, setting: Setting) {
        let set = Set { at, setting };
        let signature = Signature(key.secret().sign(&claim(self.account, &set)));
        self.keep(SignedSet { set, by: key.public(), signature });
    }

    /// Takes every value of `other` newer than the one its device holds here, once each is signed by its device and
    /// timed no more than a day past this device's clock, then drops the values of devices `members` does not hold.
    /// Tells whether anything changed; on a refusal these settings are unchanged.
    pub fn merge(&mut self, other: &Settings, members: &account::State) -> Result<bool, String> {
        if other.account != self.account {
            return Err(format!(
                "settings of account {} are not account {}'s",
                other.account.short(),
                self.account.short()
            ));
        }
        let latest = crate::now() + MAX_CLOCK_AHEAD_MS;
        let mut merged = self.clone();
        for theirs in &other.sets {
            if merged.sets.contains(theirs) {
                continue;
            }
            if theirs.set.at > latest {
                return Err(format!(
                    "a setting by {} is timed more than a day past this device's clock; check the clock",
                    theirs.by
                ));
            }
            theirs
                .by
                .endpoint()
                .verify(&claim(self.account, &theirs.set), &theirs.signature.0)
                .map_err(|_| format!("a setting is not signed by {}", theirs.by))?;
            merged.keep(theirs.clone());
        }
        merged.retain_members(members);
        let changed = merged != *self;
        *self = merged;
        Ok(changed)
    }

    /// Drops the values of every device `members` does not hold.
    pub fn retain_members(&mut self, members: &account::State) {
        self.sets.retain(|held| members.is_member(&held.by));
    }

    /// Keeps `signed` in place of its device's value of its setting, unless that one is newer.
    fn keep(&mut self, signed: SignedSet) {
        let same = |held: &SignedSet| held.by == signed.by && held.kind() == signed.kind();
        match self.sets.iter_mut().find(|held| same(held)) {
            Some(held) if held.order() < signed.order() => *held = signed,
            Some(_) => {}
            None => self.sets.push(signed),
        }
    }

    /// The newest value of each setting.
    fn newest(&self) -> Vec<&Setting> {
        let mut newest: Vec<&SignedSet> = Vec::new();
        for signed in &self.sets {
            match newest.iter_mut().find(|known| known.kind() == signed.kind()) {
                Some(known) if known.order() < signed.order() => *known = signed,
                Some(_) => {}
                None => newest.push(signed),
            }
        }
        newest.into_iter().map(|signed| &signed.set.setting).collect()
    }

    pub fn current(&self) -> Current {
        let mut current = Current::default();
        for setting in self.newest() {
            match setting.clone() {
                Setting::SectionOrder(order) => current.section_order = order,
                Setting::Collapsed(collapsed) => current.collapsed = collapsed,
                Setting::AgentFlags(flags) => current.agent_flags = flags,
            }
        }
        current
    }

    /// Sets, as `key`'s, each setting these settings hold and `next` lacks.
    pub fn carry_into(&self, next: &mut Settings, key: &DeviceKey) {
        for setting in self.newest() {
            if !next.sets.iter().any(|held| held.kind() == discriminant(setting)) {
                next.set(key, setting.clone());
            }
        }
    }
}

fn claim(account: AccountId, set: &Set) -> Vec<u8> {
    [SET_LABEL, account.as_bytes(), &serde_json::to_vec(set).expect("a setting serializes")].concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Account, Change};

    fn order(names: &[&str]) -> Setting {
        Setting::SectionOrder(names.iter().map(|name| name.to_string()).collect())
    }

    #[test]
    fn agent_flags_split_as_shell_words_and_refuse_quoting_that_does_not_close() {
        let flags = |text: &str| serde_json::from_value::<AgentFlags>(serde_json::json!(text));
        let typed = flags(" --allow-dangerously-skip-permissions --append-system-prompt 'be brief' ").expect("splits");
        assert_eq!(typed.args(), ["--allow-dangerously-skip-permissions", "--append-system-prompt", "be brief"]);
        let saved = serde_json::to_value(&typed).expect("serializes");
        assert_eq!(saved, "--allow-dangerously-skip-permissions --append-system-prompt 'be brief'");
        assert!(flags("--model 'opus").is_err());
    }

    #[test]
    fn the_newest_value_stands_whole_and_a_forged_or_former_members_value_does_not() {
        let (laptop, phone) = (DeviceKey::generate(), DeviceKey::generate());
        let mut account = Account::first(laptop.public());
        let (invite, secret, _) = account.invite(&laptop);
        account.entries.push(Account::join(account.id(), invite, &secret, &phone, "phone".into()));
        let members = account.state();
        let (mut on_laptop, mut on_phone) = (Settings::new(account.id()), Settings::new(account.id()));
        on_laptop.set(&laptop, order(&["acme", "willow"]));
        on_laptop.set(&laptop, Setting::Collapsed(vec!["acme".into()]));
        on_phone.merge(&on_laptop, &members).expect("merge");
        on_phone.set(&phone, order(&["willow", "acme"]));
        assert_eq!(on_laptop.merge(&on_phone, &members), Ok(true));
        assert_eq!(on_laptop.current(), on_phone.current());
        assert_eq!(on_laptop.current().section_order, ["willow", "acme"], "the phone set it later");
        assert_eq!(on_laptop.current().collapsed, ["acme"]);

        let mut forged = on_phone.clone();
        forged.sets[0].set.setting = order(&["forged"]);
        assert!(on_laptop.merge(&forged, &members).expect_err("forged").contains("not signed by"));

        account.append(&laptop, Change::Remove { device: phone.public() });
        on_laptop.merge(&on_phone, &account.state()).expect("merge");
        assert_eq!(on_laptop.current().section_order, ["acme", "willow"], "the removed phone's order is gone");
    }

    #[test]
    fn a_device_moving_accounts_brings_only_what_the_next_account_lacks() {
        let laptop = DeviceKey::generate();
        let mut left = Settings::new(Account::first(laptop.public()).id());
        left.set(&laptop, order(&["acme"]));
        left.set(&laptop, Setting::Collapsed(vec!["acme".into()]));
        let mut next = Settings::new(Account::fresh(laptop.public()).id());
        next.set(&laptop, order(&["willow"]));
        left.carry_into(&mut next, &laptop);
        assert_eq!(
            next.current(),
            Current { section_order: vec!["willow".into()], collapsed: vec!["acme".into()], ..Current::default() }
        );
    }
}

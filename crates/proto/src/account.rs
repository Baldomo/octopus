//! An account: the devices that vouch for each other, kept as a log of signed entries every device and trusting
//! server merges. Merging is a union, so every holder of the same entries reads the same members and servers.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use iroh::SecretKey;
use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::device::{DeviceKey, PublicKey};
use crate::remote::{self, RelayUrl};
use crate::schema::{Shape, arg_shape, spelled};

/// How long an invite stays open after it is appended.
pub const INVITE_LIFETIME_MS: i64 = 10 * 60 * 1000;
/// A log past this many entries is refused, which bounds what a peer can make a server hold.
pub const MAX_ENTRIES: usize = 4096;
/// How far past the merging device's clock an entry may be timed; a later one is refused.
pub const MAX_CLOCK_AHEAD_MS: i64 = 24 * 60 * 60 * 1000;
/// A device's name is one line of at most this many characters, so it reads whole wherever a device is listed.
pub const NAME_CHARS: usize = 64;
const ID_LABEL: &[u8] = b"octopus account";
const ENTRY_LABEL: &[u8] = b"octopus account entry";
const JOIN_LABEL: &[u8] = b"octopus account join";
const NONCE_BYTES: usize = 16;

/// A SHA-256, spelled in hex: an account's id, or an entry's hash.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

pub type AccountId = Digest;

impl Digest {
    fn of(parts: &[&[u8]]) -> Self {
        let bytes = digest(&SHA256, &parts.concat());
        Self(bytes.as_ref().try_into().expect("a SHA-256 is 32 bytes"))
    }

    /// The first eight hex digits, which a person reads an account by.
    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Digest {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(text).map_err(|error| format!("{text:?}: {error}"))?;
        Ok(Self(bytes.try_into().map_err(|_| format!("{text:?} is not 32 bytes of hex"))?))
    }
}

spelled!(Digest, Link);
arg_shape!(Shape::Text => Digest, Account, Signed, Link);

/// An ed25519 signature, spelled in hex.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(iroh::Signature);

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0.to_bytes()))
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(self.0.to_bytes()))
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = hex::decode(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)?;
        let bytes = bytes.try_into().map_err(|_| serde::de::Error::custom("a signature is 64 bytes"))?;
        Ok(Self(iroh::Signature::from_bytes(&bytes)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    /// One place in the account for whoever holds the secret of `key`, until `expires_at`.
    Invite {
        key: PublicKey,
        expires_at: i64,
    },
    /// The appending device takes the place of the invite whose key is `invite`; `proof` is that key's signature.
    Join {
        invite: PublicKey,
        name: String,
        proof: Signature,
    },
    Remove {
        device: PublicKey,
    },
    /// The appending device removes itself for the account `to`.
    Leave {
        to: AccountId,
    },
    /// The appending device's name.
    Name {
        name: String,
    },
    /// A server the account reaches, under a name; `host` is the device on whose machine it runs, or null.
    ListServer {
        server: PublicKey,
        relay: RelayUrl,
        name: String,
        host: Option<PublicKey>,
    },
    ForgetServer {
        server: PublicKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The entries its device knew of with none after them.
    pub parents: Vec<Digest>,
    /// When it was appended, in milliseconds since 1970.
    pub at: i64,
    pub change: Change,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed {
    pub entry: Entry,
    pub by: PublicKey,
    pub signature: Signature,
}

impl Signed {
    pub fn hash(&self) -> Digest {
        Digest::of(&[&serde_json::to_vec(self).expect("an entry serializes")])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub creator: PublicKey,
    /// Hex; empty for a device's first account.
    pub nonce: String,
    pub entries: Vec<Signed>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub key: PublicKey,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub key: PublicKey,
    pub relay: RelayUrl,
    pub name: String,
    /// The device whose machine runs it.
    pub host: Option<PublicKey>,
}

/// What an account's log comes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub members: Vec<Member>,
    pub servers: Vec<Listed>,
    /// Each invite no join has taken: its entry, its key and when it expires.
    pub open_invites: Vec<(Digest, PublicKey, i64)>,
    /// Each device that left, and the account it left for.
    pub left: Vec<(PublicKey, AccountId)>,
}

impl State {
    pub fn is_member(&self, key: &PublicKey) -> bool {
        self.members.iter().any(|member| member.key == *key)
    }
}

pub fn is_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= NAME_CHARS && !name.chars().any(char::is_control)
}

impl Account {
    /// A device's first account, whose id its key alone decides.
    pub fn first(creator: PublicKey) -> Self {
        Self { creator, nonce: String::new(), entries: Vec::new() }
    }

    /// A one-device account no earlier one shares an id with.
    pub fn fresh(creator: PublicKey) -> Self {
        let mut nonce = [0u8; NONCE_BYTES];
        SystemRandom::new().fill(&mut nonce).expect("the system RNG works");
        Self { creator, nonce: hex::encode(nonce), entries: Vec::new() }
    }

    pub fn id(&self) -> AccountId {
        Digest::of(&[ID_LABEL, self.creator.as_bytes(), self.nonce.as_bytes()])
    }

    /// The same account with none of its entries: what a server that trusted a device before accounts holds.
    pub fn bare(&self) -> Self {
        Self { creator: self.creator, nonce: self.nonce.clone(), entries: Vec::new() }
    }

    fn heads(&self) -> Vec<Digest> {
        let named: std::collections::HashSet<Digest> =
            self.entries.iter().flat_map(|signed| signed.entry.parents.iter().copied()).collect();
        let mut heads: Vec<Digest> =
            self.entries.iter().map(Signed::hash).filter(|hash| !named.contains(hash)).collect();
        heads.sort();
        heads
    }

    /// When an entry appended now is timed: later than every entry this log holds.
    fn next_at(&self) -> i64 {
        let latest = self.entries.iter().map(|signed| signed.entry.at + 1).max().unwrap_or(i64::MIN);
        crate::now().max(latest)
    }

    /// Appends `change` as `key`'s, after every entry this log holds and later than any of them, and returns its
    /// hash.
    pub fn append(&mut self, key: &DeviceKey, change: Change) -> Digest {
        let at = self.next_at();
        self.append_at(key, at, change)
    }

    fn append_at(&mut self, key: &DeviceKey, at: i64, change: Change) -> Digest {
        let signed = sign(self.id(), key, Entry { parents: self.heads(), at, change });
        let hash = signed.hash();
        self.entries.push(signed);
        hash
    }

    /// Appends an invite and returns its entry's hash, the secret a joining device proves, and when it expires.
    pub fn invite(&mut self, key: &DeviceKey) -> (Digest, SecretKey, i64) {
        let secret = SecretKey::generate();
        let at = self.next_at();
        let expires_at = at + INVITE_LIFETIME_MS;
        let change = Change::Invite { key: secret.public().into(), expires_at };
        (self.append_at(key, at, change), secret, expires_at)
    }

    /// The entry by which `key` takes the place of the invite `invite` names, proved with its `secret`.
    pub fn join(id: AccountId, invite: Digest, secret: &SecretKey, key: &DeviceKey, name: String) -> Signed {
        let proof = Signature(secret.sign(&join_claim(id, &key.public())));
        let change = Change::Join { invite: secret.public().into(), name, proof };
        sign(id, key, Entry { parents: vec![invite], at: crate::now(), change })
    }

    /// Takes every entry of `other` this log lacks, once each is signed by its device and names only entries the
    /// two logs hold. Returns how many it took; on a refusal this log is unchanged.
    pub fn merge(&mut self, other: &Account) -> Result<usize, String> {
        if other.id() != self.id() {
            return Err(format!("account {} is not account {}", other.id().short(), self.id().short()));
        }
        let too_many = || format!("an account holds at most {MAX_ENTRIES} entries");
        if other.entries.len() > MAX_ENTRIES {
            return Err(too_many());
        }
        let latest = crate::now() + MAX_CLOCK_AHEAD_MS;
        let mut merged = self.entries.clone();
        let mut seen: std::collections::HashSet<Digest> = self.entries.iter().map(Signed::hash).collect();
        for signed in &other.entries {
            if seen.insert(signed.hash()) {
                if signed.entry.at > latest {
                    return Err(format!(
                        "entry {} is timed more than a day past this device's clock; check the clock",
                        signed.hash().short()
                    ));
                }
                self.check_signature(signed)?;
                merged.push(signed.clone());
            }
        }
        if merged.len() > MAX_ENTRIES {
            return Err(too_many());
        }
        if let Some(orphan) =
            merged.iter().find(|signed| signed.entry.parents.iter().any(|parent| !seen.contains(parent)))
        {
            return Err(format!("entry {} names an entry neither log holds", orphan.hash().short()));
        }
        let taken = merged.len() - self.entries.len();
        self.entries = merged;
        Ok(taken)
    }

    /// The account `other` holds, its entries checked as `merge` checks them.
    pub fn checked(other: &Account) -> Result<Self, String> {
        let mut account = other.bare();
        account.merge(other)?;
        Ok(account)
    }

    fn check_signature(&self, signed: &Signed) -> Result<(), String> {
        signed
            .by
            .endpoint()
            .verify(&entry_claim(self.id(), &signed.entry), &signed.signature.0)
            .map_err(|_| format!("entry {} is not signed by {}", signed.hash().short(), signed.by))
    }

    pub fn state(&self) -> State {
        Graph::new(self).state()
    }
}

fn entry_claim(id: AccountId, entry: &Entry) -> Vec<u8> {
    [ENTRY_LABEL, &id.0, &serde_json::to_vec(entry).expect("an entry serializes")].concat()
}

fn sign(id: AccountId, key: &DeviceKey, entry: Entry) -> Signed {
    let signature = Signature(key.secret().sign(&entry_claim(id, &entry)));
    Signed { entry, by: key.public(), signature }
}

fn join_claim(id: AccountId, joiner: &PublicKey) -> Vec<u8> {
    [JOIN_LABEL, &id.0, joiner.as_bytes()].concat()
}

/// A set of entries, by their place in topological order.
#[derive(Clone)]
struct Set(Vec<u64>);

impl Set {
    fn empty(len: usize) -> Self {
        Self(vec![0; len.div_ceil(64)])
    }

    fn insert(&mut self, at: usize) {
        self.0[at / 64] |= 1 << (at % 64);
    }

    fn has(&self, at: usize) -> bool {
        self.0[at / 64] & (1 << (at % 64)) != 0
    }

    fn union(&mut self, other: &Self) {
        self.0.iter_mut().zip(&other.0).for_each(|(mine, theirs)| *mine |= theirs);
    }
}

/// The log in topological order, each entry with every entry before it, and what each entry comes to.
///
/// Every decision rests only on entries before its own, or on removals, which are decided by their own past. So one
/// pass in topological order decides each removal first, within the entries before it, and then every other entry over
/// the whole log, each from decisions already made.
struct Graph<'a> {
    id: AccountId,
    creator: PublicKey,
    entries: Vec<(&'a Signed, Digest)>,
    index: HashMap<Digest, usize>,
    before: Vec<Set>,
    /// Each device's joins, and its removals: the removes naming it and its own leaves.
    joins: HashMap<PublicKey, Vec<usize>>,
    removals: HashMap<PublicKey, Vec<usize>>,
    /// The join that takes each invite, by place: the earliest well-formed one.
    takes: HashMap<usize, usize>,
    /// Whether each removal counts, by its own past.
    stands: Vec<bool>,
    /// Whether each join and other entry counts over the whole log.
    whole: Vec<bool>,
}

impl<'a> Graph<'a> {
    fn new(account: &'a Account) -> Self {
        let mut pending: Vec<(&Signed, Digest)> =
            account.entries.iter().map(|signed| (signed, signed.hash())).collect();
        pending.sort_by_key(|(_, hash)| *hash);
        let mut entries = Vec::with_capacity(pending.len());
        let mut placed: HashMap<Digest, usize> = HashMap::new();
        while !pending.is_empty() {
            let (ready, waiting): (Vec<_>, Vec<_>) = pending
                .into_iter()
                .partition(|(signed, _)| signed.entry.parents.iter().all(|parent| placed.contains_key(parent)));
            if ready.is_empty() {
                break;
            }
            for (signed, hash) in ready {
                placed.insert(hash, entries.len());
                entries.push((signed, hash));
            }
            pending = waiting;
        }
        let len = entries.len();
        let mut before: Vec<Set> = Vec::with_capacity(len);
        for (signed, _) in &entries {
            let mut set = Set::empty(len);
            for parent in &signed.entry.parents {
                let at = placed[parent];
                set.insert(at);
                set.union(&before[at]);
            }
            before.push(set);
        }
        let mut graph = Self {
            id: account.id(),
            creator: account.creator,
            entries,
            index: placed,
            before,
            joins: HashMap::new(),
            removals: HashMap::new(),
            takes: HashMap::new(),
            stands: vec![false; len],
            whole: Vec::new(),
        };
        for at in 0..len {
            let signed = graph.entries[at].0;
            match &signed.entry.change {
                Change::Join { .. } if graph.well_formed(at) => {
                    graph.joins.entry(signed.by).or_default().push(at);
                    let Some((invite, _)) = graph.invite_of(at) else { continue };
                    let earliest = graph.takes.get(&invite).is_none_or(|&taken| graph.order(at) < graph.order(taken));
                    if earliest {
                        graph.takes.insert(invite, at);
                    }
                }
                Change::Remove { device } => graph.removals.entry(*device).or_default().push(at),
                Change::Leave { .. } => graph.removals.entry(signed.by).or_default().push(at),
                _ => {}
            }
        }
        for at in 0..len {
            if matches!(graph.entries[at].0.entry.change, Change::Remove { .. } | Change::Leave { .. }) {
                let within = &graph.before[at];
                let counts = graph.tally(Some(within), Some(&graph.behind(at)), at);
                graph.stands[at] = graph.well_formed(at) && graph.member_at(Some(within), &counts, at);
            }
        }
        graph.whole = graph.tally(None, None, len);
        graph
    }

    /// Where an entry falls among others: by its time, then its hash.
    fn order(&self, at: usize) -> (i64, Digest) {
        (self.entries[at].0.entry.at, self.entries[at].1)
    }

    /// The joins and invites whose counts decide whether `at`'s device was a member when it appended it: the joins of
    /// its device it names, each one's invite, and so on back through each inviter.
    fn behind(&self, at: usize) -> Set {
        let mut needed = Set::empty(self.entries.len());
        let mut pending = vec![at];
        while let Some(next) = pending.pop() {
            let by = self.entries[next].0.by;
            for &join in self.joins.get(&by).into_iter().flatten() {
                if !self.before[next].has(join) || needed.has(join) {
                    continue;
                }
                needed.insert(join);
                if let Some((invite, _)) = self.invite_of(join)
                    && !needed.has(invite)
                {
                    needed.insert(invite);
                    pending.push(invite);
                }
            }
        }
        needed
    }

    /// Whether each join and other entry among the first `len` that `only` holds counts, considering only the
    /// removals `within` holds; with neither, every entry over the whole log.
    fn tally(&self, within: Option<&Set>, only: Option<&Set>, len: usize) -> Vec<bool> {
        let mut counts = vec![false; len];
        for at in 0..len {
            if only.is_some_and(|only| !only.has(at)) {
                continue;
            }
            counts[at] = match self.entries[at].0.entry.change {
                Change::Remove { .. } => false,
                Change::Join { .. } => self.join_counts(within, &counts, at),
                _ => self.well_formed(at) && self.member_at(within, &counts, at),
            };
        }
        counts
    }

    /// Later than every entry it names; an invite open for exactly its lifetime; a name one line of at most
    /// `NAME_CHARS`; a join naming its invite among its parents, proved with that invite's secret before the invite
    /// expires. A join is timed by the joining device's clock, which may trail its invite's; the server holding the
    /// invite admits only the first.
    fn well_formed(&self, at: usize) -> bool {
        let (signed, _) = self.entries[at];
        let entry = &signed.entry;
        let later = entry.parents.iter().all(|parent| self.entries[self.index[parent]].0.entry.at < entry.at);
        (later || matches!(entry.change, Change::Join { .. }))
            && match &entry.change {
                Change::Invite { expires_at, .. } => *expires_at == entry.at + INVITE_LIFETIME_MS,
                Change::Join { invite, name, proof } => {
                    is_name(name)
                        && self.invite_of(at).is_some_and(|(_, expires_at)| {
                            entry.at <= expires_at
                                && invite.endpoint().verify(&join_claim(self.id, &signed.by), &proof.0).is_ok()
                        })
                }
                Change::Name { name } | Change::ListServer { name, .. } => is_name(name),
                _ => true,
            }
    }

    /// The invite among a join's parents whose key the join names, and when it expires.
    fn invite_of(&self, at: usize) -> Option<(usize, i64)> {
        let (signed, _) = self.entries[at];
        let Change::Join { invite, .. } = &signed.entry.change else { return None };
        signed.entry.parents.iter().map(|parent| self.index[parent]).find_map(|parent| {
            match &self.entries[parent].0.entry.change {
                Change::Invite { key, expires_at } if key == invite => Some((parent, *expires_at)),
                _ => None,
            }
        })
    }

    /// Whether the entry counts: a removal by its own past, any other entry over the whole log.
    fn counts(&self, at: usize) -> bool {
        match self.entries[at].0.entry.change {
            Change::Remove { .. } => self.stands[at],
            _ => self.whole[at],
        }
    }

    /// Whether the entry's device was a member when it appended it, by `counts`, what the entries before it come to.
    /// A membership runs from the device's creation or a counting join the entry names, until a counting removal
    /// names that start; the entry stands only when every such removal names the entry too.
    fn member_at(&self, within: Option<&Set>, counts: &[bool], at: usize) -> bool {
        let by = self.entries[at].0.by;
        (by == self.creator && !self.ended(within, by, None, Some(at)))
            || self
                .joins
                .get(&by)
                .into_iter()
                .flatten()
                .any(|&join| self.before[at].has(join) && counts[join] && !self.ended(within, by, Some(join), Some(at)))
    }

    /// Whether a counting removal of `device`, among those `within` holds, names the membership that began at
    /// `start` (none for the creator's) and does not name `at`.
    fn ended(&self, within: Option<&Set>, device: PublicKey, start: Option<usize>, at: Option<usize>) -> bool {
        self.removals.get(&device).into_iter().flatten().any(|&removal| {
            Some(removal) != at
                && within.is_none_or(|within| within.has(removal))
                && start.is_none_or(|start| self.before[removal].has(start))
                && at.is_none_or(|at| !self.before[removal].has(at))
                && self.stands[removal]
        })
    }

    /// A join counts when it takes a counting invite, no removal of its device runs alongside it, and no removal of
    /// its inviter names the invite but not the join.
    fn join_counts(&self, within: Option<&Set>, counts: &[bool], at: usize) -> bool {
        let Some((invite, _)) = self.invite_of(at).filter(|_| self.well_formed(at)) else { return false };
        let (by, inviter) = (self.entries[at].0.by, self.entries[invite].0.by);
        let alongside = self.removals.get(&by).into_iter().flatten().any(|&removal| {
            within.is_none_or(|within| within.has(removal))
                && !self.before[at].has(removal)
                && !self.before[removal].has(at)
                && self.stands[removal]
        });
        self.takes.get(&invite) == Some(&at)
            && counts[invite]
            && !alongside
            && !self.ended(within, inviter, Some(invite), Some(at))
    }

    /// Whether the device's newest membership is still open at the end of the log.
    fn is_member(&self, device: PublicKey) -> bool {
        (device == self.creator && !self.ended(None, device, None, None))
            || self
                .joins
                .get(&device)
                .into_iter()
                .flatten()
                .any(|&join| self.whole[join] && !self.ended(None, device, Some(join), None))
    }

    fn state(&self) -> State {
        let mut names = HashMap::from([(self.creator, (String::new(), (i64::MIN, Digest([0; 32]))))]);
        let mut servers = HashMap::new();
        let (mut invites, mut taken, mut left) = (Vec::new(), std::collections::HashSet::new(), Vec::new());
        for (at, (signed, hash)) in self.entries.iter().enumerate().filter(|&(at, _)| self.counts(at)) {
            let order = self.order(at);
            match &signed.entry.change {
                Change::ListServer { server, relay, name, host } => {
                    let listed = Listed { key: *server, relay: relay.clone(), name: name.clone(), host: *host };
                    keep_newest(&mut servers, *server, Some(listed), order);
                }
                Change::ForgetServer { server } => keep_newest(&mut servers, *server, None, order),
                Change::Invite { key, expires_at } => invites.push((*hash, *key, *expires_at)),
                Change::Join { invite, name, .. } => {
                    taken.insert(*invite);
                    keep_newest(&mut names, signed.by, name.clone(), order);
                }
                Change::Name { name } => keep_newest(&mut names, signed.by, name.clone(), order),
                Change::Leave { to } => left.push((signed.by, *to)),
                Change::Remove { .. } => {}
            }
        }
        let mut members: Vec<Member> = names
            .into_iter()
            .filter(|(key, _)| self.is_member(*key))
            .map(|(key, (name, _))| Member { key, name })
            .collect();
        members.sort_by(|a, b| (&a.name, a.key.as_bytes()).cmp(&(&b.name, b.key.as_bytes())));
        let mut servers: Vec<Listed> = servers.into_values().filter_map(|(listed, _)| listed).collect();
        servers.sort_by(|a, b| (&a.name, a.key.as_bytes()).cmp(&(&b.name, b.key.as_bytes())));
        invites.retain(|(hash, key, _)| {
            let by = self.entries[self.index[hash]].0.by;
            !taken.contains(key) && members.iter().any(|member| member.key == by)
        });
        State { members, servers, open_invites: invites, left }
    }
}

fn keep_newest<V>(map: &mut HashMap<PublicKey, (V, (i64, Digest))>, key: PublicKey, value: V, order: (i64, Digest)) {
    if map.get(&key).is_none_or(|(_, known)| order > *known) {
        map.insert(key, (value, order));
    }
}

/// Everything a device needs to join an account: the invite's entry and secret, the account, and the key and relay
/// of the server that holds the invite. Spelled
/// `octopus://<server key>?join=<secret>&account=<id>&invite=<entry hash>&relay=<relay URL>`.
#[derive(Clone)]
pub struct Link {
    pub server: PublicKey,
    pub relay: RelayUrl,
    pub account: AccountId,
    pub invite: Digest,
    pub secret: SecretKey,
}

const SCHEME: &str = "octopus";

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut url = url::Url::parse(&format!("{SCHEME}://{}", self.server.hex())).expect("a hex host is a URL");
        url.query_pairs_mut()
            .append_pair("join", &hex::encode(self.secret.to_bytes()))
            .append_pair("account", &self.account.to_string())
            .append_pair("invite", &self.invite.to_string())
            .append_pair("relay", self.relay.as_str());
        f.write_str(url.as_str())
    }
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Link({}, {}, {})", self.account.short(), self.server, self.relay)
    }
}

impl PartialEq for Link {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl FromStr for Link {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let shape =
            format!("a link is {SCHEME}://<server key>?join=<secret>&account=<id>&invite=<hash>&relay=<relay URL>");
        let url = url::Url::parse(text.trim()).map_err(|error| format!("{shape}: {error}"))?;
        let bare = url.username().is_empty() && url.password().is_none() && url.port().is_none();
        let server = match (url.scheme(), url.host_str(), url.path(), url.fragment()) {
            (SCHEME, Some(server), "", None) if bare => PublicKey::from_hex(server)?,
            _ => return Err(shape),
        };
        let (mut secret, mut account, mut invite, mut relay) = (None, None, None, None);
        for (name, value) in url.query_pairs() {
            let twice = match name.as_ref() {
                "join" => {
                    let bytes: [u8; 32] = hex::decode(value.as_ref())
                        .ok()
                        .and_then(|bytes| bytes.try_into().ok())
                        .ok_or("a link's join is 32 bytes of hex")?;
                    secret.replace(SecretKey::from_bytes(&bytes)).is_some()
                }
                "account" => account.replace(value.parse()?).is_some(),
                "invite" => invite.replace(value.parse()?).is_some(),
                "relay" => relay.replace(remote::relay(&value)?).is_some(),
                other => return Err(format!("a link has no {other}; {shape}")),
            };
            if twice {
                return Err(format!("a link names one {name}"));
            }
        }
        match (secret, account, invite, relay) {
            (Some(secret), Some(account), Some(invite), Some(relay)) => {
                Ok(Self { server, relay, account, invite, secret })
            }
            (secret, account, invite, relay) => {
                let missing = [
                    ("join", secret.is_none()),
                    ("account", account.is_none()),
                    ("invite", invite.is_none()),
                    ("relay", relay.is_none()),
                ];
                let missing: Vec<&str> =
                    missing.into_iter().filter_map(|(name, absent)| absent.then_some(name)).collect();
                Err(format!("the link names no {}", missing.join(" and no ")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay() -> RelayUrl {
        "https://relay.example./".parse().expect("a relay URL")
    }

    /// `joiner` joins `account` through an invite `inviter` appends.
    fn link(account: &mut Account, inviter: &DeviceKey, joiner: &DeviceKey, name: &str) {
        let (invite, secret, _) = account.invite(inviter);
        let join = Account::join(account.id(), invite, &secret, joiner, name.into());
        account.entries.push(join);
    }

    fn names(account: &Account) -> Vec<String> {
        account.state().members.into_iter().map(|member| member.name).collect()
    }

    #[test]
    fn a_device_joins_through_one_invite_and_its_servers_reach_every_member() {
        let (laptop, phone, tablet) = (DeviceKey::generate(), DeviceKey::generate(), DeviceKey::generate());
        let mut account = Account::first(laptop.public());
        account.append(&laptop, Change::Name { name: "laptop".into() });
        let (invite, secret, _) = account.invite(&laptop);
        account.entries.push(Account::join(account.id(), invite, &secret, &phone, "phone".into()));
        account.entries.push(Account::join(account.id(), invite, &secret, &tablet, "tablet".into()));
        let state = account.state();
        assert_eq!(state.members.len(), 2, "an invite admits one device: {:?}", state.members);
        let joined = state.members.iter().find(|member| member.key != laptop.public()).expect("a joined device").key;
        let joined = [&phone, &tablet].into_iter().find(|key| key.public() == joined).expect("one of the two");
        let server = DeviceKey::generate().public();
        account.append(joined, Change::ListServer { server, relay: relay(), name: "mac".into(), host: None });
        let state = account.state();
        assert!(state.is_member(&laptop.public()) && state.open_invites.is_empty());
        assert_eq!(state.servers.iter().map(|listed| listed.name.as_str()).collect::<Vec<_>>(), ["mac"]);
        assert_eq!(Account::checked(&account).expect("a log of signed entries checks"), account);
    }

    #[test]
    fn a_chain_of_throwaway_keys_that_never_joined_decides_at_once_and_counts_for_nothing() {
        let laptop = DeviceKey::generate();
        let mut account = Account::first(laptop.public());
        let mut inviter = DeviceKey::generate();
        for _ in 0..60 {
            let next = DeviceKey::generate();
            for _ in 0..2 {
                link(&mut account, &inviter, &next, "junk");
            }
            inviter = next;
        }
        account.append(&inviter, Change::Remove { device: laptop.public() });
        let started = std::time::Instant::now();
        assert!(account.state().is_member(&laptop.public()), "no throwaway key is a member");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
    }

    #[test]
    fn a_member_that_floods_the_log_with_removals_is_decided_at_once() {
        let (laptop, phone) = (DeviceKey::generate(), DeviceKey::generate());
        let mut account = Account::first(laptop.public());
        link(&mut account, &laptop, &phone, "phone");
        for _ in 0..200 {
            account.append(&phone, Change::Invite { key: DeviceKey::generate().public(), expires_at: 0 });
        }
        for _ in 0..200 {
            account.append(&phone, Change::Remove { device: phone.public() });
        }
        let started = std::time::Instant::now();
        assert_eq!(names(&account), [""], "the phone removed itself");
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "took {:?}", started.elapsed());
    }

    #[test]
    fn an_entry_timed_past_the_clock_or_a_name_with_a_control_character_is_refused_or_void() {
        let (laptop, phone) = (DeviceKey::generate(), DeviceKey::generate());
        let mut ours = Account::first(laptop.public());
        link(&mut ours, &laptop, &phone, "phone");
        let mut theirs = ours.clone();
        theirs.append_at(&phone, i64::MAX, Change::Name { name: "later".into() });
        assert!(ours.merge(&theirs).expect_err("future").contains("past this device's clock"));
        ours.append(&phone, Change::Name { name: "bad\u{7}".into() });
        assert_eq!(names(&ours), ["", "phone"]);
    }

    #[test]
    fn merging_takes_what_the_other_log_lacks_and_refuses_a_forged_or_orphaned_entry() {
        let (laptop, phone) = (DeviceKey::generate(), DeviceKey::generate());
        let mut ours = Account::first(laptop.public());
        link(&mut ours, &laptop, &phone, "phone");
        let mut theirs = ours.clone();
        theirs.append(&phone, Change::Name { name: "iphone".into() });
        ours.append(&laptop, Change::Name { name: "laptop".into() });
        assert_eq!(ours.merge(&theirs), Ok(1));
        assert_eq!(names(&ours), ["iphone", "laptop"]);
        let mut forged = ours.clone();
        forged.entries.last_mut().expect("an entry").by = DeviceKey::generate().public();
        assert!(ours.merge(&forged).expect_err("forged").contains("is not signed by"));
        let mut orphaned = ours.bare();
        orphaned.entries.push(ours.entries.last().expect("an entry").clone());
        assert!(Account::checked(&orphaned).expect_err("orphaned").contains("names an entry neither log holds"));
        assert!(ours.merge(&Account::fresh(laptop.public())).is_err());
    }

    #[test]
    fn a_removed_device_is_out_with_whatever_it_did_that_its_removal_did_not_see() {
        let (laptop, phone, thief) = (DeviceKey::generate(), DeviceKey::generate(), DeviceKey::generate());
        let mut account = Account::first(laptop.public());
        link(&mut account, &laptop, &phone, "phone");
        let server = DeviceKey::generate().public();
        account.append(&phone, Change::ListServer { server, relay: relay(), name: "kept".into(), host: None });
        let mut stolen = account.clone();
        link(&mut stolen, &phone, &thief, "thief");
        stolen.append(&thief, Change::Remove { device: laptop.public() });
        account.append(&laptop, Change::Remove { device: phone.public() });
        let mut owner = account.clone();
        owner.merge(&stolen).expect("merge");
        let state = owner.state();
        assert!(
            state.members.is_empty(),
            "the thief's removal stands by its own past, so all three are out: {:?}",
            state.members
        );
        assert_eq!(state.servers.len(), 1, "what the phone did before its removal stands");
        let late = owner.append(&phone, Change::Name { name: "back".into() });
        assert!(!owner.state().is_member(&phone.public()), "{late:?} after the removal counts for nothing");
    }

    #[test]
    fn a_removed_device_keeps_no_device_it_brought_in_and_its_late_leave_counts_for_nothing() {
        let (laptop, phone) = (DeviceKey::generate(), DeviceKey::generate());
        let (first, second) = (DeviceKey::generate(), DeviceKey::generate());
        let mut account = Account::first(laptop.public());
        link(&mut account, &laptop, &phone, "phone");
        let mut stolen = account.clone();
        link(&mut stolen, &phone, &first, "first");
        link(&mut stolen, &first, &second, "second");
        account.append(&laptop, Change::Remove { device: phone.public() });
        stolen.append(&phone, Change::Leave { to: Account::fresh(phone.public()).id() });
        account.merge(&stolen).expect("merge");
        assert_eq!(names(&account), [""], "only the laptop, unnamed, stays");
        assert!(account.state().left.is_empty(), "a leave the removal did not see counts for nothing");
    }

    #[test]
    fn a_link_of_a_removed_device_admits_nobody_its_removal_did_not_see() {
        let (laptop, phone, thief) = (DeviceKey::generate(), DeviceKey::generate(), DeviceKey::generate());
        let mut account = Account::first(laptop.public());
        link(&mut account, &laptop, &phone, "phone");
        let (invite, secret, _) = account.invite(&phone);
        account.append(&laptop, Change::Remove { device: phone.public() });
        assert!(account.state().open_invites.is_empty(), "a removed device's invite is no longer open");
        account.entries.push(Account::join(account.id(), invite, &secret, &thief, "thief".into()));
        assert!(!account.state().is_member(&thief.public()));
    }

    #[test]
    fn removals_that_never_saw_their_targets_join_decide_the_same_in_any_order() {
        let creator = DeviceKey::generate();
        let [x, w, d, v] = [(); 4].map(|()| DeviceKey::generate());
        let mut base = Account::first(creator.public());
        let invites: Vec<_> = (0..3)
            .map(|_| {
                let (invite, secret, _) = base.invite(&creator);
                (invite, secret)
            })
            .collect();
        let joined = |account: &mut Account, (invite, secret): &(Digest, SecretKey), key: &DeviceKey| {
            account.entries.push(Account::join(account.id(), *invite, secret, key, String::new()));
        };
        let (mut one, mut two, mut three) = (base.clone(), base.clone(), base.clone());
        joined(&mut one, &invites[0], &x);
        one.append(&x, Change::Remove { device: d.public() });
        joined(&mut two, &invites[1], &w);
        two.append(&w, Change::Remove { device: x.public() });
        joined(&mut three, &invites[2], &d);
        let (invite, secret, _) = three.invite(&d);
        joined(&mut three, &(invite, secret), &v);
        three.append(&v, Change::Remove { device: w.public() });
        let (mut forward, mut backward) = (base.clone(), base);
        for branch in [&one, &two, &three] {
            forward.merge(branch).expect("merge");
        }
        for branch in [&three, &two, &one] {
            backward.merge(branch).expect("merge");
        }
        assert_eq!(forward.state().members, backward.state().members);
        assert_eq!(names(&forward), [""], "each removal stands by its own past, so only the creator stays");
    }

    #[test]
    fn a_device_that_left_or_was_removed_rejoins_through_a_fresh_link() {
        let (laptop, phone) = (DeviceKey::generate(), DeviceKey::generate());
        let mut account = Account::first(laptop.public());
        link(&mut account, &laptop, &phone, "phone");
        account.append(&phone, Change::Leave { to: Account::fresh(phone.public()).id() });
        assert!(!account.state().is_member(&phone.public()));
        link(&mut account, &laptop, &phone, "phone");
        assert!(account.state().is_member(&phone.public()), "a join after the leave starts a new membership");
        account.append(&laptop, Change::Remove { device: phone.public() });
        assert!(!account.state().is_member(&phone.public()));
        link(&mut account, &laptop, &phone, "phone");
        assert!(account.state().is_member(&phone.public()), "and after a removal too");
    }

    #[test]
    fn two_devices_that_remove_each_other_at_once_are_both_out() {
        let (laptop, phone, tablet) = (DeviceKey::generate(), DeviceKey::generate(), DeviceKey::generate());
        let mut account = Account::first(laptop.public());
        link(&mut account, &laptop, &phone, "phone");
        link(&mut account, &laptop, &tablet, "tablet");
        let mut other = account.clone();
        account.append(&laptop, Change::Remove { device: phone.public() });
        other.append(&phone, Change::Remove { device: laptop.public() });
        account.merge(&other).expect("merge");
        assert_eq!(names(&account), ["tablet"]);
    }

    #[test]
    fn the_newest_server_entry_lists_it_and_a_leave_names_the_next_account() {
        let laptop = DeviceKey::generate();
        let mut account = Account::first(laptop.public());
        let server = DeviceKey::generate().public();
        let host = Some(laptop.public());
        for name in ["laptop", "desk"] {
            account.append(&laptop, Change::ListServer { server, relay: relay(), name: name.into(), host });
        }
        let listed = account.state().servers.remove(0);
        assert_eq!((listed.name.as_str(), listed.host), ("desk", host));
        account.append(&laptop, Change::ForgetServer { server });
        assert!(account.state().servers.is_empty());
        let next = Account::fresh(laptop.public()).id();
        account.append(&laptop, Change::Leave { to: next });
        assert_eq!(account.state().left, [(laptop.public(), next)]);
        assert!(account.state().members.is_empty());
    }

    #[test]
    fn a_link_round_trips_as_text_and_names_what_is_wrong_with_a_bad_one() {
        let link = Link {
            server: DeviceKey::generate().public(),
            relay: relay(),
            account: Account::first(DeviceKey::generate().public()).id(),
            invite: Digest([7; 32]),
            secret: SecretKey::generate(),
        };
        assert_eq!(link.to_string().parse::<Link>(), Ok(link.clone()));
        let text = link.to_string();
        assert!(text.replace("&invite=", "&other=").parse::<Link>().expect_err("other").contains("has no other"));
        let cut = &text[..text.find("&account").expect("account")];
        assert!(cut.parse::<Link>().expect_err("cut").contains("names no account and no invite and no relay"));
    }
}

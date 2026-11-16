//! The agent sessions the demo computer replays, recorded as beats of output and status, and the player that replays
//! them at their recorded pace.

use std::sync::Arc;
use std::time::Duration;

use octopus_proto::types::{AgentStatus, PtyId};

use super::Demo;
use super::terminal::Turn;

pub(super) const PROMPT: &str = "\x1b[2m›\x1b[0m ";

pub(super) enum Beat {
    Pause(u64),
    Say(&'static str),
    /// The user's request, typed at the prompt one key at a time.
    Type(&'static str),
    Status(AgentStatus),
    /// Waits for a line typed into the terminal, then goes on with `no` for one starting with n, else with `yes`.
    Ask {
        yes: &'static [Beat],
        no: &'static [Beat],
    },
}

use AgentStatus::{Idle, Waiting, Working};
use Beat::{Ask, Pause, Say, Status, Type};

const TYPING: Duration = Duration::from_millis(45);

const CHART: &[Beat] = &[
    Say("\x1b[1m◆ agent\x1b[0m \x1b[2m· weekly-tide-chart\x1b[0m\n\n"),
    Pause(2500),
    Type("Add a weekly tide chart to the harbor page"),
    Say("\n\n"),
    Status(Working),
    Pause(1800),
    Say("\x1b[36m●\x1b[0m Reading src/tides.js\n"),
    Pause(1200),
    Say("\x1b[36m●\x1b[0m Reading src/forecast.js\n"),
    Pause(2600),
    Say("  The page lists today's tides from\n"),
    Say("  data/harbor.csv, and the whole week is\n"),
    Say("  parsed already, so the chart only needs\n"),
    Say("  drawing.\n\n"),
    Pause(3000),
    Say("\x1b[36m●\x1b[0m Writing src/chart.js\n"),
    Pause(900),
    Say("\x1b[32m  + export function weekChart(days) {\n"),
    Pause(300),
    Say("  +   const tides = days.flatMap(\n"),
    Say("  +     (day) => day.tides);\n"),
    Pause(300),
    Say("  +   const step = WIDTH / tides.length;\n"),
    Pause(300),
    Say("  +   return line(tides, step);\n"),
    Say("  + }\x1b[0m\n\n"),
    Pause(2400),
    Say("\x1b[36m●\x1b[0m Editing src/tides.js  \x1b[32m+2\x1b[0m\n"),
    Pause(1500),
    Say("\x1b[36m●\x1b[0m Editing src/index.html  \x1b[32m+1\x1b[0m\n\n"),
    Pause(2200),
    Say("\x1b[36m●\x1b[0m Running make test\n"),
    Pause(3500),
    Say("  \x1b[32m✓\x1b[0m groups tides by day\n"),
    Pause(400),
    Say("  \x1b[32m✓\x1b[0m keeps each tide's offset\n"),
    Pause(400),
    Say("  \x1b[32m✓\x1b[0m reads a bare time as +00:00\n"),
    Pause(400),
    Say("  \x1b[32m✓\x1b[0m skips blank rows\n"),
    Say("  \x1b[2m4 passed\x1b[0m\n\n"),
    Pause(2000),
    Say("\x1b[36m●\x1b[0m Checking localhost:4100\n\n"),
    Pause(3500),
    Say("Done. The harbor page draws the week's tides\n"),
    Say("as one line under today's table. The chart\n"),
    Say("is in the Browser tab.\n\n"),
    Status(Idle),
    Say(PROMPT),
];

const SPRING: &[Beat] = &[
    Type("Make spring tides above 4 m stand out"),
    Say("\n\n"),
    Status(Working),
    Pause(2000),
    Say("\x1b[36m●\x1b[0m Reading docs/notes.md\n"),
    Pause(2400),
    Say("  The harbor office says tides above 4 m\n"),
    Say("  flood the slipway.\n\n"),
    Pause(2000),
    Say("\x1b[36m●\x1b[0m Editing src/chart.js  \x1b[32m+4\x1b[0m \x1b[31m-1\x1b[0m\n"),
    Pause(1800),
    Say("\x1b[36m●\x1b[0m Editing src/style.css  \x1b[32m+5\x1b[0m\n\n"),
    Pause(3000),
    Say("Done. Tides above 4 m get a darker mark and\n"),
    Say("a dashed line at the slipway's height.\n\n"),
    Status(Idle),
    Say(PROMPT),
];

/// The weekly chart task's recordings, which it replays in turn, idle between them.
pub(super) const CHART_TASK: &[&[Beat]] = &[CHART, SPRING];
/// How long an agent rests idle between recordings.
const REST: Duration = Duration::from_secs(45);

const OFFSET_APPLIED: &[Beat] = &[
    Say("\n"),
    Status(Working),
    Pause(1500),
    Say("\x1b[36m●\x1b[0m Rewriting data/harbor.csv  \x1b[2m18 rows\x1b[0m\n"),
    Pause(2600),
    Say("\x1b[36m●\x1b[0m Running make test\n"),
    Pause(3000),
    Say("  \x1b[32m✓\x1b[0m keeps each tide's offset\n"),
    Pause(400),
    Say("  \x1b[32m✓\x1b[0m reads a bare time as +00:00\n"),
    Say("  \x1b[2m4 passed\x1b[0m\n\n"),
    Pause(1500),
    Say("Done. Harbor times follow the cove's own\n"),
    Say("offset, the summer ones too.\n\n"),
    Status(Idle),
    Say(PROMPT),
];

const OFFSET_DECLINED: &[Beat] = &[
    Say("\n"),
    Pause(800),
    Say("Left data/harbor.csv as it is. The fix in\n"),
    Say("src/forecast.js reads an offset wherever a\n"),
    Say("row carries one.\n\n"),
    Status(Idle),
    Say(PROMPT),
];

pub(super) const OFFSET: &[Beat] = &[
    Say("\x1b[1m◆ agent\x1b[0m \x1b[2m· harbor-time-offset\x1b[0m\n\n"),
    Pause(1500),
    Type("Harbor times run an hour late in summer"),
    Say("\n\n"),
    Status(Working),
    Pause(2000),
    Say("\x1b[36m●\x1b[0m Reading src/forecast.js\n"),
    Pause(1400),
    Say("\x1b[36m●\x1b[0m Searching \"offset\"  \x1b[2m3 matches\x1b[0m\n"),
    Pause(3000),
    Say("  Summer rows of data/harbor.csv carry no\n"),
    Say("  offset, so forecast.js reads them as\n"),
    Say("  +00:00 and each lands an hour late.\n\n"),
    Pause(2500),
    Say("\x1b[36m●\x1b[0m Editing src/forecast.js  \x1b[32m+3\x1b[0m \x1b[31m-1\x1b[0m\n\n"),
    Pause(2000),
    Say("\x1b[33m?\x1b[0m Write the offset into every row of\n"),
    Say("  data/harbor.csv? \x1b[1m[y/n]\x1b[0m "),
    Status(Waiting),
    Ask { yes: OFFSET_APPLIED, no: OFFSET_DECLINED },
];

pub(super) const TESTS: &[Beat] = &[
    Say("\x1b[1m◆ agent\x1b[0m \x1b[2m· forecast-tests\x1b[0m\n\n"),
    Type("Write tests for the forecast parser"),
    Say("\n\n"),
    Say("\x1b[36m●\x1b[0m Reading src/forecast.js\n"),
    Say("\x1b[36m●\x1b[0m Writing tests/harness.js\n"),
    Say("\x1b[36m●\x1b[0m Writing tests/forecast.test.js\n"),
    Say("\x1b[32m  + test(\"groups tides by day\", …)\n"),
    Say("  + test(\"keeps each tide's offset\", …)\n"),
    Say("  + test(\"reads a bare time as +00:00\", …)\n"),
    Say("  + test(\"skips blank rows\", …)\x1b[0m\n\n"),
    Say("\x1b[36m●\x1b[0m Running make test\n"),
    Say("  \x1b[2m4 passed\x1b[0m\n\n"),
    Say("Added four tests for the parser, blank rows\n"),
    Say("and missing offsets among them. The report\n"),
    Say("is in the Browser tab.\n\n"),
    Say(PROMPT),
];

pub(super) const FRESH: &str = "\x1b[1m◆ agent\x1b[0m\n\n\x1b[2mA fresh session. Type a request below.\x1b[0m\n\n";

const REPLY: &[Beat] = &[
    Status(Working),
    Pause(1200),
    Say("\x1b[36m●\x1b[0m This is the demo computer: its agents\n"),
    Say("  replay recorded sessions, so nothing runs\n"),
    Say("  and no file changes. Open a shell with +\n"),
    Say("  to try a few commands there.\n\n"),
    Status(Idle),
    Say(PROMPT),
];

/// The text a recording leaves on its terminal, for a session that ended before the demo started.
pub(super) fn transcript(beats: &[Beat]) -> String {
    let said = beats.iter().map(|beat| match beat {
        Say(text) => text.to_string(),
        Type(request) => format!("{PROMPT}{request}"),
        Pause(_) | Status(_) | Ask { .. } => String::new(),
    });
    said.collect()
}

/// Replays `recordings` on the agent terminal `pty` in turn and over again, resting idle after each, and starting the
/// screen afresh before each round but the first.
pub(super) async fn cycle(demo: Arc<Demo>, pty: PtyId, recordings: &'static [&'static [Beat]]) {
    loop {
        for beats in recordings {
            if !replay(Arc::clone(&demo), pty.clone(), beats).await {
                return;
            }
            tokio::time::sleep(REST).await;
        }
        if !demo.world().clear(&pty) {
            return;
        }
    }
}

/// Answers a request the agent did not ask for with what the demo agent can do.
pub(super) async fn reply(demo: Arc<Demo>, pty: PtyId) {
    replay(demo, pty, REPLY).await;
}

/// Resolves to whether the terminal outlived the recording.
pub(super) async fn replay(demo: Arc<Demo>, pty: PtyId, beats: &'static [Beat]) -> bool {
    let Some(stage) = demo.world().stage(&pty) else { return false };
    let _stage = stage.lock().await;
    demo.world().turn(&pty, Turn::Playing);
    let mut beats = beats.iter();
    while let Some(beat) = beats.next() {
        let alive = match beat {
            Pause(millis) => {
                tokio::time::sleep(Duration::from_millis(*millis)).await;
                true
            }
            Say(text) => demo.world().say(&pty, text),
            Type(request) => type_request(&demo, &pty, request).await,
            Status(status) => demo.world().status(&pty, *status),
            Ask { yes, no } => {
                let Some(answered) = demo.world().ask(&pty) else { return false };
                let Ok(answer) = answered.await else { return false };
                beats = if answer.trim_start().to_lowercase().starts_with('n') { no.iter() } else { yes.iter() };
                true
            }
        };
        if !alive {
            return false;
        }
    }
    demo.world().turn(&pty, Turn::Ready);
    true
}

async fn type_request(demo: &Demo, pty: &PtyId, request: &str) -> bool {
    if !demo.world().say(pty, PROMPT) {
        return false;
    }
    for key in request.chars() {
        tokio::time::sleep(TYPING).await;
        if !demo.world().say(pty, key.encode_utf8(&mut [0; 4])) {
            return false;
        }
    }
    true
}

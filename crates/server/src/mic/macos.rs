//! The Octopus Microphone: one loopback HAL device per machine. Programs record from its input; the server plays the
//! listening terminal's audio into its output, which the device hands back to them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::Context;
use nix::unistd::Pid;
use octopus_proto::types::{MIC_DEVICE, MIC_FORMAT, PtyId};

use super::Shared;
use crate::host::HostClient;
use crate::{lock, report};

/// The user's own default input while the Octopus Microphone stands in for it.
const RESTORE_INPUT: &str = "restore-input";
/// A process's running input changes on that process's object alone, so one poll reads them all.
const POLL: Duration = Duration::from_millis(200);

pub struct Platform {
    inner: Arc<Inner>,
}

struct Inner {
    device: ca::Device,
    dir: PathBuf,
    shared: Arc<Shared>,
    /// This server's terminals; another server's may record from the same device.
    terminals: Mutex<HashSet<PtyId>>,
    host: HostClient,
    /// The program each terminal started, which leads the terminal's session.
    leaders: Mutex<HashMap<PtyId, Pid>>,
    /// The listening terminals in the order they started; the device carries the first.
    order: Mutex<Vec<PtyId>>,
    output: Mutex<Option<ca::Output>>,
    /// The terminals whose lock a client connected over QUIC holds.
    remote_held: Mutex<HashSet<PtyId>>,
    /// The terminals whose holder is sending its microphone.
    streaming: Mutex<HashSet<PtyId>>,
}

impl Platform {
    pub fn start(dir: &Path, terminal_host: &Path, shared: Arc<Shared>) -> anyhow::Result<Self> {
        let device = ca::Device::find(MIC_DEVICE.uid)?
            .with_context(|| format!("no audio device {}; reinstall the server to install it", MIC_DEVICE.uid))?;
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let inner = Arc::new(Inner {
            host: HostClient::at(terminal_host),
            leaders: Mutex::default(),
            device,
            dir: dir.to_path_buf(),
            shared,
            terminals: Mutex::default(),
            order: Mutex::default(),
            output: Mutex::default(),
            remote_held: Mutex::default(),
            streaming: Mutex::default(),
        });
        let watched = Arc::downgrade(&inner);
        std::thread::spawn(move || watch(&watched));
        Ok(Self { inner })
    }

    pub fn prepare(&self, id: &PtyId) -> anyhow::Result<Vec<(String, String)>> {
        lock(&self.inner.terminals).insert(id.clone());
        Ok(Vec::new())
    }

    pub fn started(&self, id: &PtyId) {
        self.inner.learn_leader(id);
    }

    pub fn adopt(&self, terminals: &[PtyId]) {
        lock(&self.inner.terminals).extend(terminals.iter().cloned());
        terminals.iter().for_each(|id| self.inner.learn_leader(id));
    }

    pub fn remove(&self, id: &PtyId) {
        lock(&self.inner.terminals).remove(id);
        lock(&self.inner.leaders).remove(id);
    }

    pub fn remote_held(&self, terminals: HashSet<PtyId>) {
        *lock(&self.inner.remote_held) = terminals;
    }

    pub fn streaming(&self, id: &PtyId, streaming: bool) {
        let mut sending = lock(&self.inner.streaming);
        if streaming {
            sending.insert(id.clone())
        } else {
            sending.remove(id)
        };
    }
}

/// The one thread that switches the default input, so a switch that fails, or a saved input that was gone, is
/// tried again each poll. A failure is reported when it differs from the last.
fn watch(inner: &Weak<Inner>) {
    let mut last_failure = String::new();
    while let Some(inner) = inner.upgrade() {
        let failure = inner.poll().err().map(|error| format!("{error:#}")).unwrap_or_default();
        if !failure.is_empty() && failure != last_failure {
            report!("the Octopus Microphone: {failure}");
        }
        last_failure = failure;
        drop(inner);
        std::thread::sleep(POLL);
    }
}

impl Inner {
    /// The device stands in for the default input exactly while a terminal it would carry listens and its remote
    /// holder sends.
    fn poll(&self) -> anyhow::Result<()> {
        let (recording, skipped) = ca::recording_processes(self.device).context("list the processes recording")?;
        let listening = self.listen(&recording);
        let carried = &*lock(&self.streaming) & &*lock(&self.remote_held);
        let switched = if listening.iter().any(|id| carried.contains(id)) {
            self.take_input()
        } else {
            restore_input(self.device, &self.restore_path())
        };
        switched.context("switch the default input")?;
        skipped.map_or(Ok(()), |error| Err(error.context("read an audio process")))
    }

    fn listen(&self, recording: &[ca::Recording]) -> HashSet<PtyId> {
        let terminals = lock(&self.terminals).clone();
        let remote_held = lock(&self.remote_held).clone();
        let now = listeners(recording, &terminals, &remote_held, |pid| self.terminal_of(pid));
        let first = {
            let mut order = lock(&self.order);
            advance(&mut order, &now);
            order.first().cloned()
        };
        let listening = now.clone();
        self.shared.set_listening(now);
        let mut output = lock(&self.output);
        match (first, output.as_mut()) {
            (Some(first), Some(running)) => running.carry(first),
            (Some(first), None) => match ca::Output::start(self.device, Arc::clone(&self.shared), first) {
                Ok(started) => *output = Some(started),
                Err(error) => report!("play into the Octopus Microphone: {error:#}"),
            },
            (None, _) => *output = None,
        }
        listening
    }

    fn terminal_of(&self, pid: Pid) -> Option<PtyId> {
        let leader = nix::unistd::getsid(Some(pid)).ok()?;
        lock(&self.leaders).iter().find(|(_, known)| **known == leader).map(|(id, _)| id.clone())
    }

    /// A terminal whose program has already exited has no leader to learn.
    fn learn_leader(&self, id: &PtyId) {
        match self.host.program(id) {
            Ok(Some(program)) => {
                lock(&self.leaders).insert(id.clone(), Pid::from_raw(program));
            }
            Ok(None) => {}
            Err(error) => report!("find the program of {id}: {error}"),
        }
    }

    fn restore_path(&self) -> PathBuf {
        self.dir.join(RESTORE_INPUT)
    }

    /// The user's choice is saved before it is replaced, and kept while the device stands in for it.
    fn take_input(&self) -> anyhow::Result<()> {
        let current = ca::default_input()?;
        if current == self.device {
            return Ok(());
        }
        let saved = self.restore_path();
        if !saved.exists() {
            let uid = current.uid()?;
            std::fs::write(&saved, uid).with_context(|| format!("write {}", saved.display()))?;
        }
        ca::set_default_input(self.device)
    }
}

/// Gives back the input saved at `saved` while `device` is still the default; a default the user changed since
/// stays. A saved input that is gone stays saved until it is back.
fn restore_input(device: ca::Device, saved: &Path) -> anyhow::Result<()> {
    let uid = match std::fs::read_to_string(saved) {
        Ok(uid) => uid,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("read {}", saved.display())),
    };
    if ca::default_input()? == device {
        let Some(chosen) = ca::Device::find(&uid)? else { return Ok(()) };
        ca::set_default_input(chosen)?;
    }
    std::fs::remove_file(saved).with_context(|| format!("remove {}", saved.display()))
}

/// Gives back the input a stopped server's Octopus Microphone stood in for.
pub fn give_back_input(dir: &Path) -> anyhow::Result<()> {
    match ca::Device::find(MIC_DEVICE.uid)? {
        Some(device) => restore_input(device, &dir.join(RESTORE_INPUT)),
        None => Ok(()),
    }
}

/// This server's listening terminals: those with a program recording from the device, and those whose remote holder
/// stands in for whatever input their program records from.
fn listeners(
    recording: &[ca::Recording],
    terminals: &HashSet<PtyId>,
    remote_held: &HashSet<PtyId>,
    terminal_of: impl Fn(Pid) -> Option<PtyId>,
) -> HashSet<PtyId> {
    recording
        .iter()
        .filter_map(|recording| Some((terminal_of(recording.pid)?, recording.from_device)))
        .filter(|(id, from_device)| terminals.contains(id) && (*from_device || remote_held.contains(id)))
        .map(|(id, _)| id)
        .collect()
}

/// Keeps the terminals still listening in the order they started, then appends those that just started.
fn advance(order: &mut Vec<PtyId>, now: &HashSet<PtyId>) {
    order.retain(|id| now.contains(id));
    let mut started: Vec<&PtyId> = now.iter().filter(|id| !order.contains(id)).collect();
    started.sort();
    order.extend(started.into_iter().cloned());
}

/// `MIC_FORMAT` samples as the device's Float32.
fn to_float(pcm: &[u8], out: &mut [f32]) {
    let (samples, _) = pcm.as_chunks::<{ MIC_FORMAT.bytes_per_sample }>();
    for (sample, bytes) in out.iter_mut().zip(samples) {
        *sample = f32::from(i16::from_le_bytes(*bytes)) / 32768.0;
    }
}

/// CoreAudio's C API, kept to the calls this module makes.
#[allow(unsafe_code, reason = "CoreAudio and CoreFoundation have only C entry points")]
mod ca {
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::sync::{Arc, Mutex};

    use anyhow::{Context, bail};
    use nix::unistd::Pid;
    use octopus_proto::types::{MIC_FORMAT, PtyId};

    use super::{Shared, to_float};
    use crate::lock;
    use crate::mic::unpoisoned;

    type ObjectId = u32;
    type OsStatus = i32;
    type CfString = *const c_void;

    const fn code(text: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*text)
    }

    const SYSTEM: ObjectId = 1;
    const UNKNOWN: ObjectId = 0;
    const SCOPE_GLOBAL: u32 = code(b"glob");
    const SCOPE_INPUT: u32 = code(b"inpt");
    const TRANSLATE_UID: u32 = code(b"uidd");
    const DEFAULT_INPUT: u32 = code(b"dIn ");
    const DEVICE_UID: u32 = code(b"uid ");
    const PROCESSES: u32 = code(b"prs#");
    const PROCESS_PID: u32 = code(b"ppid");
    const PROCESS_RUNNING_INPUT: u32 = code(b"piri");
    const PROCESS_DEVICES: u32 = code(b"pdv#");
    const IO_PROC_STREAM_USAGE: u32 = code(b"suse");
    const UTF8: u32 = 0x0800_0100;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[repr(C)]
    struct Buffer {
        channels: u32,
        bytes: u32,
        data: *mut c_void,
    }

    #[repr(C)]
    struct BufferList {
        count: u32,
        buffers: [Buffer; 1],
    }

    type IoProc = extern "C" fn(
        ObjectId,
        *const c_void,
        *const BufferList,
        *const c_void,
        *mut BufferList,
        *const c_void,
        *mut c_void,
    ) -> OsStatus;
    type IoProcId = *mut c_void;

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object: ObjectId,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
        ) -> OsStatus;
        fn AudioObjectGetPropertyData(
            object: ObjectId,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> OsStatus;
        fn AudioObjectSetPropertyData(
            object: ObjectId,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: u32,
            data: *const c_void,
        ) -> OsStatus;
        fn AudioDeviceCreateIOProcID(
            device: ObjectId,
            proc: IoProc,
            client: *mut c_void,
            id: *mut IoProcId,
        ) -> OsStatus;
        fn AudioDeviceDestroyIOProcID(device: ObjectId, id: IoProcId) -> OsStatus;
        fn AudioDeviceStart(device: ObjectId, id: IoProcId) -> OsStatus;
        fn AudioDeviceStop(device: ObjectId, id: IoProcId) -> OsStatus;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(allocator: *const c_void, text: *const c_char, encoding: u32) -> CfString;
        fn CFStringGetCString(text: CfString, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(object: *const c_void);
    }

    /// `kAudioHardwareBadObjectError`: the object is gone.
    const BAD_OBJECT: OsStatus = code(b"!obj") as OsStatus;

    #[derive(Debug)]
    struct Status(OsStatus);

    impl std::fmt::Display for Status {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "CoreAudio status {}", self.0)
        }
    }

    impl std::error::Error for Status {}

    fn check(what: &str, status: OsStatus) -> anyhow::Result<()> {
        if status != 0 {
            return Err(Status(status)).context(what.to_string());
        }
        Ok(())
    }

    fn address(selector: u32, scope: u32) -> Address {
        Address { selector, scope, element: 0 }
    }

    /// One fixed-size value of a property.
    fn get<T: Copy>(object: ObjectId, address: &Address, qualifier: Option<&CfString>) -> anyhow::Result<T> {
        let mut value = std::mem::MaybeUninit::<T>::uninit();
        let mut size = u32::try_from(size_of::<T>()).expect("a property value fits u32");
        let (qualifier_size, qualifier) = match qualifier {
            Some(text) => (size_of::<CfString>() as u32, std::ptr::from_ref(text).cast()),
            None => (0, std::ptr::null()),
        };
        // SAFETY: `value` has room for `size` bytes, and a qualifier, when given, is one live CFString.
        let status = unsafe {
            AudioObjectGetPropertyData(object, address, qualifier_size, qualifier, &mut size, value.as_mut_ptr().cast())
        };
        check("read a property", status)?;
        if size as usize != size_of::<T>() {
            bail!("a property came back {size} bytes, not {}", size_of::<T>());
        }
        // SAFETY: CoreAudio filled all of it.
        Ok(unsafe { value.assume_init() })
    }

    fn objects(object: ObjectId, address: &Address) -> anyhow::Result<Vec<ObjectId>> {
        let mut size = 0u32;
        // SAFETY: `size` is a valid out pointer.
        check("size a list", unsafe {
            AudioObjectGetPropertyDataSize(object, address, 0, std::ptr::null(), &mut size)
        })?;
        let mut list = vec![UNKNOWN; size as usize / size_of::<ObjectId>()];
        let mut size = u32::try_from(list.len() * size_of::<ObjectId>()).expect("a list fits u32");
        // SAFETY: `list` has room for `size` bytes.
        check("read a list", unsafe {
            AudioObjectGetPropertyData(object, address, 0, std::ptr::null(), &mut size, list.as_mut_ptr().cast())
        })?;
        list.truncate(size as usize / size_of::<ObjectId>());
        Ok(list)
    }

    struct OwnedString(CfString);

    impl OwnedString {
        fn new(text: &str) -> anyhow::Result<Self> {
            let text = CString::new(text)?;
            // SAFETY: `text` is a NUL-terminated UTF-8 string.
            let made = unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) };
            if made.is_null() {
                bail!("make a CFString of {text:?}");
            }
            Ok(Self(made))
        }

        fn text(&self) -> anyhow::Result<String> {
            let mut buffer = [0 as c_char; 1024];
            // SAFETY: the buffer's length is passed with it, and CoreFoundation NUL-terminates on success.
            let copied = unsafe { CFStringGetCString(self.0, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) };
            if copied == 0 {
                bail!("read a CFString");
            }
            // SAFETY: the copy succeeded, so the buffer holds a NUL-terminated string.
            Ok(unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy().into_owned())
        }
    }

    impl Drop for OwnedString {
        fn drop(&mut self) {
            // SAFETY: this owns one reference, from a create or a copy-rule property read.
            unsafe { CFRelease(self.0) };
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Device(ObjectId);

    impl Device {
        pub fn find(uid: &str) -> anyhow::Result<Option<Self>> {
            let uid = OwnedString::new(uid)?;
            let found: ObjectId = get(SYSTEM, &address(TRANSLATE_UID, SCOPE_GLOBAL), Some(&uid.0))?;
            Ok((found != UNKNOWN).then_some(Self(found)))
        }

        pub fn uid(self) -> anyhow::Result<String> {
            let uid: CfString = get(self.0, &address(DEVICE_UID, SCOPE_GLOBAL), None)?;
            OwnedString(uid).text()
        }
    }

    pub fn default_input() -> anyhow::Result<Device> {
        get(SYSTEM, &address(DEFAULT_INPUT, SCOPE_GLOBAL), None).map(Device)
    }

    pub fn set_default_input(device: Device) -> anyhow::Result<()> {
        let size = size_of::<ObjectId>() as u32;
        // SAFETY: the data is one AudioObjectID, as the property takes.
        check("set the default input", unsafe {
            AudioObjectSetPropertyData(
                SYSTEM,
                &address(DEFAULT_INPUT, SCOPE_GLOBAL),
                0,
                std::ptr::null(),
                size,
                std::ptr::from_ref(&device.0).cast(),
            )
        })
    }

    pub struct Recording {
        pub pid: Pid,
        /// Whether it records from the Octopus Microphone rather than another input.
        pub from_device: bool,
    }

    /// The processes now recording from any input, beside the first failure to read one that has not exited.
    /// Process objects exist from macOS 14.
    pub fn recording_processes(device: Device) -> anyhow::Result<(Vec<Recording>, Option<anyhow::Error>)> {
        let processes = objects(SYSTEM, &address(PROCESSES, SCOPE_GLOBAL)).context("list audio processes")?;
        let recording = |process: ObjectId| -> anyhow::Result<Option<Recording>> {
            let running: u32 = get(process, &address(PROCESS_RUNNING_INPUT, SCOPE_GLOBAL), None)?;
            if running == 0 {
                return Ok(None);
            }
            let from_device = objects(process, &address(PROCESS_DEVICES, SCOPE_INPUT))?.contains(&device.0);
            let pid: i32 = get(process, &address(PROCESS_PID, SCOPE_GLOBAL), None)?;
            Ok(Some(Recording { pid: Pid::from_raw(pid), from_device }))
        };
        let exited =
            |error: &anyhow::Error| error.downcast_ref::<Status>().is_some_and(|status| status.0 == BAD_OBJECT);
        let (mut recordings, mut skipped) = (Vec::new(), None);
        for process in processes {
            match recording(process) {
                Ok(found) => recordings.extend(found),
                Err(error) if exited(&error) => {}
                Err(error) => {
                    skipped.get_or_insert(error);
                }
            }
        }
        Ok((recordings, skipped))
    }

    /// The PCM one IOProc call converts at a time; a longer buffer takes several.
    const CHUNK_FRAMES: usize = 1024;

    /// What the IOProc plays: the audio waiting for one terminal. The IOProc runs in real time, so it only tries
    /// each lock and allocates nothing; a lock it misses costs that cycle silence.
    struct Feed {
        shared: Arc<Shared>,
        terminal: Mutex<PtyId>,
        pcm: Mutex<[u8; CHUNK_FRAMES * MIC_FORMAT.bytes_per_sample]>,
    }

    /// Which of a scope's streams an IOProc uses: the device has one input stream.
    #[repr(C)]
    struct StreamUsage {
        io_proc: IoProcId,
        streams: u32,
        on: [u32; 1],
    }

    /// A running IOProc playing into the device's output; dropping it stops it.
    pub struct Output {
        device: Device,
        id: IoProcId,
        feed: *const Feed,
    }

    // SAFETY: the IOProc id and the feed pointer are used only through CoreAudio calls and `Arc`, which any thread may make.
    unsafe impl Send for Output {}

    impl Output {
        pub fn start(device: Device, shared: Arc<Shared>, terminal: PtyId) -> anyhow::Result<Self> {
            let pcm = Mutex::new([0; CHUNK_FRAMES * MIC_FORMAT.bytes_per_sample]);
            let feed = Arc::into_raw(Arc::new(Feed { shared, terminal: Mutex::new(terminal), pcm }));
            let mut id: IoProcId = std::ptr::null_mut();
            // SAFETY: `feed` stays alive until the IOProc is destroyed in `drop`.
            let created = unsafe { AudioDeviceCreateIOProcID(device.0, play, feed.cast_mut().cast(), &mut id) };
            let output = Self { device, id, feed };
            check("create the IOProc", created)?;
            // The IOProc plays only: running the device's input would make the server record.
            let usage = StreamUsage { io_proc: id, streams: 1, on: [0] };
            // SAFETY: `usage` names this device's IOProc and its one input stream, as the property takes.
            check("turn the IOProc's input off", unsafe {
                AudioObjectSetPropertyData(
                    device.0,
                    &address(IO_PROC_STREAM_USAGE, SCOPE_INPUT),
                    0,
                    std::ptr::null(),
                    size_of::<StreamUsage>() as u32,
                    std::ptr::from_ref(&usage).cast(),
                )
            })?;
            // SAFETY: `id` is the IOProc just created on this device.
            check("start the IOProc", unsafe { AudioDeviceStart(device.0, id) })?;
            Ok(output)
        }

        pub fn carry(&mut self, terminal: PtyId) {
            // SAFETY: the feed lives as long as `self`.
            *lock(&unsafe { &*self.feed }.terminal) = terminal;
        }
    }

    impl Drop for Output {
        fn drop(&mut self) {
            if !self.id.is_null() {
                // SAFETY: `id` is this device's IOProc; once destroyed, CoreAudio calls it no more.
                unsafe {
                    AudioDeviceStop(self.device.0, self.id);
                    AudioDeviceDestroyIOProcID(self.device.0, self.id);
                }
            }
            // SAFETY: the pointer came from `Arc::into_raw`, and nothing uses it after the IOProc is gone.
            drop(unsafe { Arc::from_raw(self.feed) });
        }
    }

    extern "C" fn play(
        _device: ObjectId,
        _now: *const c_void,
        _input: *const BufferList,
        _input_time: *const c_void,
        output: *mut BufferList,
        _output_time: *const c_void,
        client: *mut c_void,
    ) -> OsStatus {
        // SAFETY: `client` is the live `Feed` given at creation, and `output` the device's mono Float32 buffers.
        let (feed, output) = unsafe { (&*client.cast::<Feed>(), &mut *output) };
        if output.count == 0 || output.buffers[0].data.is_null() {
            return 0;
        }
        let buffer = &mut output.buffers[0];
        let frames = buffer.bytes as usize / size_of::<f32>();
        // SAFETY: the buffer holds `frames` floats.
        let samples = unsafe { std::slice::from_raw_parts_mut(buffer.data.cast::<f32>(), frames) };
        let (Some(terminal), Some(mut pcm)) = (unpoisoned(feed.terminal.try_lock()), unpoisoned(feed.pcm.try_lock()))
        else {
            samples.fill(0.0);
            return 0;
        };
        for chunk in samples.chunks_mut(CHUNK_FRAMES) {
            let pcm = &mut pcm[..chunk.len() * MIC_FORMAT.bytes_per_sample];
            if feed.shared.try_pull(&terminal, pcm) {
                to_float(pcm, chunk);
            } else {
                chunk.fill(0.0);
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> PtyId {
        PtyId(text.into())
    }

    #[test]
    fn a_terminal_listens_from_the_device_or_from_any_input_while_a_remote_client_holds_it() {
        let terminals = HashSet::from([id("pty_a"), id("pty_b"), id("pty_c")]);
        let remote_held = HashSet::from([id("pty_b")]);
        let panels = HashMap::from([(101, "pty_a"), (201, "pty_b"), (301, "pty_c"), (401, "pty_other")]);
        let terminal_of = |pid: Pid| panels.get(&pid.as_raw()).map(|panel| id(panel));
        let recording = [(101, true), (201, false), (301, false), (401, true), (999, true)]
            .map(|(pid, from_device)| ca::Recording { pid: Pid::from_raw(pid), from_device });
        let listening = listeners(&recording, &terminals, &remote_held, terminal_of);
        assert_eq!(listening, HashSet::from([id("pty_a"), id("pty_b")]));
    }

    #[test]
    fn the_device_carries_the_terminal_that_started_listening_first() {
        let mut order = Vec::new();
        advance(&mut order, &HashSet::from([id("pty_b")]));
        advance(&mut order, &HashSet::from([id("pty_a"), id("pty_b")]));
        assert_eq!(order, [id("pty_b"), id("pty_a")]);
        advance(&mut order, &HashSet::from([id("pty_a")]));
        assert_eq!(order, [id("pty_a")]);
    }
}

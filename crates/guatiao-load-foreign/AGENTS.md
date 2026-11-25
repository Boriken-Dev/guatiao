# guatiao-load-foreign — shipped API header

Native guatiao libraries in a statically linked Linux host, run by the
system's own `ld.so` and C library on threads that C library made. Linux on
x86_64 and aarch64; empty elsewhere. Depends on `guatiao` (no features). No
features. Not published.

## API

```rust
pub struct Foreign;                 // Clone; the started C library and its workers
impl Foreign {
    pub fn start(settings: &Settings) -> Result<Foreign, Error>;
    pub fn open(&self, name: impl AsRef<OsStr>) -> Result<Library, Error>;       // settings.open_flags
    pub fn open_with(&self, name: impl AsRef<OsStr>, flags: OpenFlags) -> Result<Library, Error>;
    pub unsafe fn call(&self, job: Job, argument: *mut c_void) -> Result<(), Error>;     // a free worker
    pub unsafe fn call_on(&self, index: usize, job: Job, argument: *mut c_void) -> Result<(), Error>;
    pub fn workers(&self) -> usize;
    pub fn worker_ids(&self) -> Vec<i32>;        // kernel tids, the bootstrap thread first
    pub fn signals_taken(&self) -> SignalSet;    // handlers that changed during start (glibc: 33)
    pub fn ld_so(&self) -> &Path;
}
pub type Job = unsafe extern "C" fn(*mut c_void);

pub struct Library;                 // Debug; dropping it leaves the library open
impl Library {
    pub fn symbol(&self, name: &str) -> Result<Option<Symbol>, Error>;  // dlsym on a worker
    pub fn entry(&self) -> Result<Option<Symbol>, Error>;               // guatiao_library_entry
    pub fn declaration(&self) -> Result<Option<Vec<u8>>, Error>;        // guatiao_declares, to its terminator
    pub fn close(self) -> Result<(), Error>;                            // dlclose on a worker
    pub fn name(&self) -> &OsStr;
    pub fn handle(&self) -> *mut c_void;
    pub fn foreign(&self) -> &Foreign;
}
pub struct Symbol;                  // Copy
impl Symbol {
    pub fn address(&self) -> *mut c_void;
    pub fn size(&self) -> Option<u64>;           // st_size, None when 0
    pub unsafe fn as_job(&self) -> Job;
}

#[non_exhaustive] pub struct Settings {          // Default
    pub ld_so: Option<PathBuf>,                  // None: the helper's PT_INTERP
    pub helper: Helper,                          // Built
    pub workers: usize,                          // 1; 0 refused
    pub bootstrap_stack: usize,                  // 8 MiB
    pub stack_alignment: usize,                  // 16; a power of two >= 16
    pub worker_stack: Option<usize>,             // None: the C library's default
    pub spin: u32,                               // 2000 looks before a futex sleep
    pub blocked_signals: SignalSet,              // SignalSet::asynchronous()
    pub environment: Environment,                // WithoutLoaderVariables
    pub start_timeout: Option<Duration>,         // 10 s
    pub error_capacity: usize,                   // 1024 bytes of dlerror text
    pub declaration_window: usize,               // 4096
    pub read_unsized_declaration: bool,          // false
    pub open_flags: OpenFlags,                   // NOW | LOCAL
    pub allow_reserved_signal_handlers: bool,    // false
    pub fence_program_break: bool,               // true
}
#[non_exhaustive] pub enum Helper { Built, Path(PathBuf), Bytes(Arc<[u8]>) }   // Default Built
impl Helper {
    pub const SOURCE: &'static str;              // helper/helper.c
    pub fn built() -> Option<&'static [u8]>;
    pub fn needs(&self) -> Result<Vec<VersionNeed>, Error>;   // .gnu.version_r
    pub fn build(compiler: &Path, out: &Path) -> Result<Helper, Error>;
}
pub struct VersionNeed { pub file: String, pub version: String }
#[non_exhaustive] pub enum Environment { WithoutLoaderVariables, Inherit, Exact(Vec<OsString>) }
pub struct SignalSet;               // signals 1..=64; EMPTY, all(), asynchronous(), with, without, contains, iter, bits
pub struct OpenFlags;               // LAZY NOW NOLOAD DEEPBIND GLOBAL LOCAL NODELETE, BitOr, bits

#[non_exhaustive] pub enum Error {
    LdSo { path, reason: ImageError }, Helper { path: Option<PathBuf>, reason: ImageError },
    NoHelper, Build { status, stderr }, Setting { name, reason }, SignalInUse { signal },
    Fence(io::Error), Memory(io::Error), Thread(io::Error), StartTimedOut(Duration),
    Layout, Spawn(i32), Forked, NoWorker { index, workers }, Name(OsString),
    Open { name, message }, Close { message },
    DeclarationTooLarge { size, window }, DeclarationSizeUnknown, DeclarationUnterminated { read },
}
#[non_exhaustive] pub enum ImageError {
    Io(io::Error), NotElf, Class, Machine { found, expected }, Type(u16),
    NoInterpreter, HasInterpreter, Malformed(&'static str),
}
```

## Contract

- `start` reads and checks both files before anything runs: a missing or
  unusable `ld.so` or helper is `Error::LdSo`/`Error::Helper` with the path and
  the reason, the host untouched. After `ld.so` is entered, its own failures
  (no C library found) print and end the process, as for any program.
- The workers, the images and their memory last as long as the process: a
  C library cannot let its threads go without ending it. Dropping every
  `Foreign` leaves them parked. A failed or timed-out start leaves its thread
  parked too.
- A job runs foreign code only: host code on a worker finds the foreign C
  library's thread state where its own should be.
- `declaration()` copies `guatiao_declares` (parse with
  `guatiao::library::Declared::parse`): the size comes from the library's
  symbol table, refused above the window; a symbol with no size is refused
  unless `read_unsized_declaration`.

## Environment

- `GUATIAO_FOREIGN_HELPER_CC` (build): the C compiler for the helper, a glibc
  compiler for the target's architecture (default `cc`, only when cargo runs
  on that architecture); `none` builds with no helper, so `Settings::helper`
  must name one.

## Gotchas

- **Signals 32 and 33**: glibc installs a handler on 33 when it starts, and a
  glibc host already has one, so a glibc host (or a second `Foreign`) must set
  `allow_reserved_signal_handlers`. musl installs its cancellation handler on
  33 at its first `pthread_cancel`, replacing glibc's; a foreign `setuid`
  after that is not measured.
- **SIGKILL and SIGSTOP** stay unblocked whatever `blocked_signals` says, and
  glibc keeps 32 and 33 open.
- **`fork`**: the child has no workers; every call there is `Error::Forked`.
- **Credentials and exit**: a host `setuid` does not reach the workers; the
  foreign C library's `atexit` handlers and destructors do not run at exit.
- **The fence** maps one page where the program break ends (Linux 4.17's
  `MAP_FIXED_NOREPLACE`); both allocators then use `mmap`. Without it the two
  allocators trim each other's heap away.

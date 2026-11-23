//! Allocating/blocking codec APIs. Never open, decode, seek, encode or close on an audio callback.
//! The native backend is dynamically loaded libsndfile; it is not a Rust FLAC implementation.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
    ptr,
    sync::Arc,
};
pub type FormatError = Box<dyn std::error::Error + Send + Sync>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum AudioFormat {
    Wav,
    Aiff,
    Flac,
}
impl AudioFormat {
    fn code(self) -> i32 {
        match self {
            Self::Wav => 0x010000,
            Self::Aiff => 0x020000,
            Self::Flac => 0x170000,
        }
    }
    fn from_code(code: i32) -> Result<Self, FormatError> {
        match code & 0x0FFF0000 {
            0x010000 | 0x130000 | 0x220000 => Ok(Self::Wav),
            0x020000 => Ok(Self::Aiff),
            0x170000 => Ok(Self::Flac),
            _ => Err("codec manager supports WAV, AIFF and FLAC".into()),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Encoding {
    Pcm16,
    Pcm24,
    Float32,
}
impl Encoding {
    fn code(self) -> i32 {
        match self {
            Self::Pcm16 => 2,
            Self::Pcm24 => 3,
            Self::Float32 => 6,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct MediaInfo {
    pub format: AudioFormat,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub seekable: bool,
    pub metadata: BTreeMap<String, String>,
}
#[repr(C)]
#[derive(Default)]
struct Info {
    frames: i64,
    rate: i32,
    channels: i32,
    format: i32,
    sections: i32,
    seekable: i32,
}
#[cfg(not(windows))]
type Open = unsafe extern "C" fn(*const c_char, i32, *mut Info) -> *mut c_void;
type Close = unsafe extern "C" fn(*mut c_void) -> i32;
type Read = unsafe extern "C" fn(*mut c_void, *mut f32, i64) -> i64;
type Write = unsafe extern "C" fn(*mut c_void, *const f32, i64) -> i64;
type Seek = unsafe extern "C" fn(*mut c_void, i64, i32) -> i64;
type Check = unsafe extern "C" fn(*const Info) -> i32;
type Error = unsafe extern "C" fn(*mut c_void) -> i32;
type StrError = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type Get = unsafe extern "C" fn(*mut c_void, i32) -> *const c_char;
type Set = unsafe extern "C" fn(*mut c_void, i32, *const c_char) -> i32;
type WriteSync = unsafe extern "C" fn(*mut c_void);
struct Bindings {
    _library: libloading::Library,
    #[cfg(not(windows))]
    open: Open,
    close: Close,
    read: Read,
    write: Write,
    seek: Seek,
    check: Check,
    error: Error,
    strerror: StrError,
    get: Get,
    set: Set,
    sync: WriteSync,
    version: String,
    #[cfg(windows)]
    wide: unsafe extern "C" fn(*const u16, i32, *mut Info) -> *mut c_void,
}
const TAGS: [(&str, i32); 6] = [
    ("title", 1),
    ("artist", 4),
    ("album", 7),
    ("comment", 5),
    ("date", 6),
    ("genre", 0x10),
];
impl Bindings {
    unsafe fn load(path: &Path) -> Result<Self, FormatError> {
        unsafe {
            let library = libloading::Library::new(path)?;
            #[cfg(not(windows))]
            let open = *library.get(b"sf_open\0")?;
            let close = *library.get(b"sf_close\0")?;
            let read = *library.get(b"sf_readf_float\0")?;
            let write = *library.get(b"sf_writef_float\0")?;
            let seek = *library.get(b"sf_seek\0")?;
            let check = *library.get(b"sf_format_check\0")?;
            let error = *library.get(b"sf_error\0")?;
            let strerror = *library.get(b"sf_strerror\0")?;
            let get = *library.get(b"sf_get_string\0")?;
            let set = *library.get(b"sf_set_string\0")?;
            let sync = *library.get(b"sf_write_sync\0")?;
            let version: *const c_char =
                library.get::<unsafe extern "C" fn() -> *const c_char>(b"sf_version_string\0")?();
            let version = if version.is_null() {
                "unknown".into()
            } else {
                CStr::from_ptr(version).to_string_lossy().into_owned()
            };
            #[cfg(windows)]
            let wide = *library.get(b"sf_wchar_open\0")?;
            Ok(Self {
                _library: library,
                #[cfg(not(windows))]
                open,
                close,
                read,
                write,
                seek,
                check,
                error,
                strerror,
                get,
                set,
                sync,
                version,
                #[cfg(windows)]
                wide,
            })
        }
    }
    fn message(&self, file: *mut c_void) -> FormatError {
        unsafe {
            let p = (self.strerror)(file);
            if p.is_null() {
                "native codec operation failed".into()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned().into()
            }
        }
    }
    fn open(&self, path: &Path, mode: i32, info: &mut Info) -> Result<*mut c_void, FormatError> {
        #[cfg(not(windows))]
        let file = {
            let path = CString::new(path.to_str().ok_or("codec path must be UTF-8")?)?;
            unsafe { (self.open)(path.as_ptr(), mode, info) }
        };
        #[cfg(windows)]
        let file = {
            use std::os::windows::ffi::OsStrExt;
            let mut path: Vec<u16> = path.as_os_str().encode_wide().collect();
            if path.contains(&0) {
                return Err("path contains NUL".into());
            }
            path.push(0);
            unsafe { (self.wide)(path.as_ptr(), mode, info) }
        };
        if file.is_null() {
            Err(self.message(ptr::null_mut()))
        } else {
            Ok(file)
        }
    }
}
#[derive(Clone)]
pub struct FormatManager {
    backend: Arc<Bindings>,
}
impl FormatManager {
    pub fn native_default() -> Result<Self, FormatError> {
        let names: &[&str] = if cfg!(target_os = "windows") {
            &["libsndfile-1.dll", "sndfile.dll"]
        } else if cfg!(target_os = "macos") {
            &["libsndfile.1.dylib", "libsndfile.dylib"]
        } else {
            &["libsndfile.so.1", "libsndfile.so"]
        };
        let mut errors = vec![];
        for name in names {
            match unsafe { Self::from_library(Path::new(name)) } {
                Ok(m) => return Ok(m),
                Err(e) => errors.push(e.to_string()),
            }
        }
        Err(format!("libsndfile runtime unavailable: {}", errors.join("; ")).into())
    }
    /// # Safety
    /// The selected native library must be trusted and implement libsndfile's stable ABI.
    pub unsafe fn from_library(path: &Path) -> Result<Self, FormatError> {
        Ok(Self {
            backend: Arc::new(unsafe { Bindings::load(path) }?),
        })
    }
    pub fn backend_version(&self) -> &str {
        &self.backend.version
    }
    pub fn supports(
        &self,
        format: AudioFormat,
        encoding: Encoding,
        rate: u32,
        channels: u16,
    ) -> bool {
        if !(1..=768000).contains(&rate) || !(1..=64).contains(&channels) {
            return false;
        }
        let info = Info {
            rate: rate as i32,
            channels: channels as i32,
            format: format.code() | encoding.code(),
            ..Default::default()
        };
        unsafe { (self.backend.check)(&info) != 0 }
    }
    pub fn open(&self, path: &Path) -> Result<Reader, FormatError> {
        let mut info = Info::default();
        let file = self.backend.open(path, 0x10, &mut info)?;
        let handle = Handle {
            backend: self.backend.clone(),
            file,
        };
        if info.frames < 0
            || !(1..=64).contains(&info.channels)
            || !(1..=768000).contains(&info.rate)
        {
            return Err("invalid decoded audio metadata".into());
        }
        let format = AudioFormat::from_code(info.format)?;
        let mut metadata = BTreeMap::new();
        for (key, id) in TAGS {
            let value = unsafe { (self.backend.get)(file, id) };
            if !value.is_null() {
                metadata.insert(
                    key.into(),
                    unsafe { CStr::from_ptr(value) }
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        Ok(Reader {
            handle,
            info: MediaInfo {
                format,
                sample_rate: info.rate as u32,
                channels: info.channels as u16,
                frames: info.frames as u64,
                seekable: info.seekable != 0,
                metadata,
            },
            position: 0,
        })
    }
    pub fn create(
        &self,
        path: &Path,
        format: AudioFormat,
        encoding: Encoding,
        rate: u32,
        channels: u16,
    ) -> Result<Writer, FormatError> {
        if path.exists() {
            return Err("codec output already exists".into());
        }
        if !self.supports(format, encoding, rate, channels) {
            return Err("unsupported format/encoding/rate/channels".into());
        }
        let mut info = Info {
            rate: rate as i32,
            channels: channels as i32,
            format: format.code() | encoding.code(),
            ..Default::default()
        };
        let file = self.backend.open(path, 0x20, &mut info)?;
        Ok(Writer {
            handle: Handle {
                backend: self.backend.clone(),
                file,
            },
            channels: channels as usize,
            frames: 0,
            pcm: !matches!(encoding, Encoding::Float32),
        })
    }
    pub fn read_all(
        &self,
        path: &Path,
        max_pcm_bytes: usize,
    ) -> Result<soniclaw_io::AudioFile, FormatError> {
        let mut reader = self.open(path)?;
        let samples = usize::try_from(reader.info.frames)?
            .checked_mul(reader.info.channels as usize)
            .ok_or("decoded size overflow")?;
        let bytes = samples.checked_mul(4).ok_or("decoded size overflow")?;
        if bytes > max_pcm_bytes || bytes > 512 * 1024 * 1024 {
            return Err("decoded PCM exceeds memory budget".into());
        }
        let mut data = vec![0.0; samples];
        let channels = reader.info.channels as usize;
        let mut read = 0;
        for chunk in data.chunks_mut(8192 * channels) {
            let count = reader.read_frames(chunk)?;
            read += count * channels;
            if count * channels != chunk.len() {
                return Err("truncated audio while decoding".into());
            }
        }
        if read != samples {
            return Err("decoded length mismatch".into());
        }
        Ok(soniclaw_io::AudioFile {
            sample_rate: reader.info.sample_rate,
            channels: reader.info.channels,
            samples: data,
        })
    }
}
struct Handle {
    backend: Arc<Bindings>,
    file: *mut c_void,
}
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.file.is_null() {
            unsafe {
                (self.backend.close)(self.file);
            }
            self.file = ptr::null_mut();
        }
    }
}
pub struct Reader {
    handle: Handle,
    info: MediaInfo,
    position: u64,
}
impl Reader {
    pub fn info(&self) -> &MediaInfo {
        &self.info
    }

    pub fn position(&self) -> u64 {
        self.position
    }
    pub fn seek(&mut self, frame: u64) -> Result<u64, FormatError> {
        if !self.info.seekable || frame > self.info.frames || frame > i64::MAX as u64 {
            return Err("unsupported/out-of-range audio seek".into());
        }
        let p = unsafe { (self.handle.backend.seek)(self.handle.file, frame as i64, 0) };
        if p < 0 {
            return Err(self.handle.backend.message(self.handle.file));
        }
        self.position = p as u64;
        Ok(self.position)
    }
    pub fn read_frames(&mut self, output: &mut [f32]) -> Result<usize, FormatError> {
        output.fill(0.0);
        let ch = self.info.channels as usize;
        if !output.len().is_multiple_of(ch) || output.len() > 64 * 1024 * 1024 {
            return Err("invalid/oversized decode buffer".into());
        }
        let frames = output.len() / ch;
        let n = unsafe {
            (self.handle.backend.read)(self.handle.file, output.as_mut_ptr(), frames as i64)
        };
        if n < 0
            || n as usize > frames
            || unsafe { (self.handle.backend.error)(self.handle.file) } != 0
            || output.iter().any(|x| !x.is_finite())
        {
            output.fill(0.0);
            return Err(self.handle.backend.message(self.handle.file));
        }
        self.position = self.position.saturating_add(n as u64);
        Ok(n as usize)
    }
}
pub struct Writer {
    handle: Handle,
    channels: usize,
    frames: u64,
    pcm: bool,
}
impl Writer {
    pub fn set_metadata(&mut self, key: &str, value: &str) -> Result<(), FormatError> {
        let id = TAGS
            .iter()
            .find(|(k, _)| *k == key)
            .ok_or("unsupported metadata key")?
            .1;
        if value.len() > 65536 {
            return Err("metadata value exceeds 64 KiB".into());
        }
        let value = CString::new(value)?;
        if unsafe { (self.handle.backend.set)(self.handle.file, id, value.as_ptr()) } != 0 {
            return Err(self.handle.backend.message(self.handle.file));
        }
        Ok(())
    }
    pub fn write_frames(&mut self, data: &[f32]) -> Result<(), FormatError> {
        if !data.len().is_multiple_of(self.channels)
            || data.len() > 64 * 1024 * 1024
            || data
                .iter()
                .any(|x| !x.is_finite() || self.pcm && x.abs() > 1.0)
        {
            return Err("invalid/oversized encoded audio".into());
        }
        let frames = data.len() / self.channels;
        let n =
            unsafe { (self.handle.backend.write)(self.handle.file, data.as_ptr(), frames as i64) };
        if n != frames as i64 || unsafe { (self.handle.backend.error)(self.handle.file) } != 0 {
            return Err(self.handle.backend.message(self.handle.file));
        }
        self.frames = self.frames.saturating_add(frames as u64);
        Ok(())
    }
    /// Finalize explicitly to observe native flush/close failures. Drop only performs best-effort close.
    pub fn finish(mut self) -> Result<u64, FormatError> {
        unsafe {
            (self.handle.backend.sync)(self.handle.file);
        }
        let error = unsafe { (self.handle.backend.error)(self.handle.file) };
        let close = unsafe { (self.handle.backend.close)(self.handle.file) };
        self.handle.file = ptr::null_mut();
        if error != 0 || close != 0 {
            return Err(format!("codec finalization failed: error={error}, close={close}").into());
        }
        Ok(self.frames)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtype_and_shape_contract() {
        assert_eq!(AudioFormat::from_code(0x170002).unwrap(), AudioFormat::Flac);
        assert!(AudioFormat::from_code(0x200000).is_err());
        assert_eq!(Encoding::Pcm24.code(), 3);
    }
}

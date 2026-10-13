//! Small synchronous HTTPS transport, used only by background workers.
//! WinHTTP supplies TLS and proxy support; no HTTP client/runtime is bundled.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const MAX_URL_BYTES: usize = 8192;
const MAX_REDIRECTS: usize = 3;
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(90);

#[cfg_attr(test, derive(Debug))]
struct Url<'a> {
    host: &'a str,
    path: &'a str,
}

fn parse_url(url: &str) -> Result<Url<'_>, String> {
    // Restrict authority syntax instead of accepting a general URL parser's
    // alternate spellings, credentials, ports or escaped host names.
    if url.len() > MAX_URL_BYTES
        || url
            .bytes()
            .any(|byte| !matches!(byte, b'!'..=b'~') || matches!(byte, b'\\' | b'#'))
    {
        return Err("L’adresse de téléchargement est invalide.".into());
    }
    let authority = url
        .strip_prefix("https://")
        .ok_or_else(|| "Le téléchargement doit utiliser HTTPS.".to_string())?;
    let (host, path) = authority
        .find('/')
        .map_or((authority, "/"), |index| authority.split_at(index));
    if ![
        "api.github.com",
        "github.com",
        "release-assets.githubusercontent.com",
        "objects.githubusercontent.com",
        "raw.githubusercontent.com",
    ]
    .iter()
    .any(|allowed| host.eq_ignore_ascii_case(allowed))
    {
        return Err("L’hôte de téléchargement n’est pas autorisé.".into());
    }
    Ok(Url { host, path })
}

fn redirect_url(current: &str, location: &str, followed: usize) -> Result<String, String> {
    if followed >= MAX_REDIRECTS {
        return Err("Le téléchargement contient trop de redirections.".into());
    }
    let next = if location.starts_with('/') && !location.starts_with("//") {
        format!("https://{}{location}", parse_url(current)?.host)
    } else {
        location.to_string()
    };
    // Validate each destination before establishing another connection. Relative
    // references other than a root path are deliberately unsupported.
    parse_url(&next)?;
    Ok(next)
}

enum Sink<'a> {
    Memory(&'a mut Vec<u8>),
    File(&'a mut File),
}

impl Sink<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Memory(destination) => destination.extend_from_slice(bytes),
            Self::File(file) => file
                .write_all(bytes)
                .map_err(|_| "Impossible d’enregistrer le téléchargement.".to_string())?,
        }
        Ok(())
    }
}

fn checked_size(current: usize, received: usize, maximum: usize) -> Result<usize, String> {
    current
        .checked_add(received)
        .filter(|size| *size <= maximum)
        .ok_or_else(|| "Le téléchargement est trop volumineux.".to_string())
}

pub fn get(url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    transfer(
        url,
        max_bytes,
        &mut Sink::Memory(&mut bytes),
        METADATA_TIMEOUT,
    )?;
    Ok(bytes)
}

/// Create a new staging file and stream directly into it. An existing path is
/// never overwritten, and every failure closes/removes the partial download.
pub fn download(url: &str, destination: &Path, max_bytes: usize) -> Result<(), String> {
    parse_url(url)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "Impossible de créer le fichier de téléchargement.".to_string())?;
    let result = transfer(url, max_bytes, &mut Sink::File(&mut file), DOWNLOAD_TIMEOUT);
    finish_file_download(file, destination, result)
}

fn finish_file_download(
    file: File,
    destination: &Path,
    result: Result<(), String>,
) -> Result<(), String> {
    let result = result.and_then(|()| {
        file.sync_all()
            .map_err(|_| "Impossible d’enregistrer le téléchargement.".to_string())
    });
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

#[cfg(windows)]
fn transfer(
    url: &str,
    maximum: usize,
    sink: &mut Sink<'_>,
    timeout: Duration,
) -> Result<(), String> {
    use std::ffi::c_void;
    use std::io;
    use std::ptr::{null, null_mut};
    use std::time::Instant;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Networking::WinHttp::*;

    struct InternetHandle(*mut c_void);
    impl InternetHandle {
        fn new(pointer: *mut c_void) -> Result<Self, String> {
            if pointer.is_null() {
                Err(network_error())
            } else {
                Ok(Self(pointer))
            }
        }

        fn option(&self, option: u32, value: u32) -> Result<(), String> {
            check(unsafe {
                WinHttpSetOption(
                    self.0,
                    option,
                    (&value as *const u32).cast(),
                    std::mem::size_of::<u32>() as u32,
                )
            })
        }

        fn deadline(&self, started: Instant, timeout: Duration) -> Result<(), String> {
            let remaining = timeout
                .checked_sub(started.elapsed())
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| "Le téléchargement a dépassé le délai autorisé.".to_string())?;
            // A send can perform resolution, connection and transmission. Bound
            // all three phases, and recheck the same deadline before each read.
            let phase = (remaining.as_millis() / 3).clamp(1, 5000) as i32;
            check(unsafe { WinHttpSetTimeouts(self.0, phase.min(3000), phase, phase, phase) })?;
            self.option(WINHTTP_OPTION_RECEIVE_RESPONSE_TIMEOUT, phase as u32)
        }

        fn header(&self, query: u32) -> Result<Option<String>, String> {
            let mut size = 0;
            if unsafe {
                WinHttpQueryHeaders(self.0, query, null(), null_mut(), &mut size, null_mut())
            } == 0
                && unsafe { GetLastError() } == ERROR_WINHTTP_HEADER_NOT_FOUND
            {
                return Ok(None);
            }
            // The redirect URL is the largest header we need. Bound the UTF-16
            // allocation even if the server supplies a malformed/huge header.
            if size == 0 || size as usize > (MAX_URL_BYTES + 1) * 2 || size % 2 != 0 {
                return Err("Un en-tête de téléchargement est invalide.".into());
            }
            let mut value = vec![0_u16; size as usize / 2];
            check(unsafe {
                WinHttpQueryHeaders(
                    self.0,
                    query,
                    null(),
                    value.as_mut_ptr().cast(),
                    &mut size,
                    null_mut(),
                )
            })?;
            let length = value
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(value.len());
            String::from_utf16(&value[..length])
                .map(Some)
                .map_err(|_| "Un en-tête de téléchargement est invalide.".to_string())
        }
    }
    impl Drop for InternetHandle {
        fn drop(&mut self) {
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn check(result: i32) -> Result<(), String> {
        if result == 0 {
            Err(network_error())
        } else {
            Ok(())
        }
    }
    fn network_error() -> String {
        format!("Impossible de télécharger : {}", io::Error::last_os_error())
    }

    parse_url(url)?;
    let started = Instant::now();
    let user_agent = wide("SuperMenu");
    let verb = wide("GET");
    let session = InternetHandle::new(unsafe {
        WinHttpOpen(
            user_agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        )
    })?;
    session.option(
        WINHTTP_OPTION_SECURE_PROTOCOLS,
        WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
    )?;
    session.option(WINHTTP_OPTION_CONNECT_RETRIES, 1)?;
    let mut current = url.to_string();
    for followed in 0..=MAX_REDIRECTS {
        let parts = parse_url(&current)?;
        let host = wide(parts.host);
        let path = wide(parts.path);
        let connection =
            InternetHandle::new(unsafe { WinHttpConnect(session.0, host.as_ptr(), 443, 0) })?;
        let request = InternetHandle::new(unsafe {
            WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            )
        })?;
        request.option(
            WINHTTP_OPTION_DISABLE_FEATURE,
            WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_AUTHENTICATION,
        )?;
        request.option(
            WINHTTP_OPTION_AUTOLOGON_POLICY,
            WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
        )?;
        request.option(WINHTTP_OPTION_MAX_RESPONSE_HEADER_SIZE, 16_384)?;
        request.deadline(started, timeout)?;
        // GitHub's JSON API requires no credentials for public releases. Asset
        // and language downloads retain their default response representation.
        let headers = if parts.host.eq_ignore_ascii_case("api.github.com") {
            wide("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n")
        } else {
            vec![0]
        };
        check(unsafe {
            WinHttpSendRequest(
                request.0,
                headers.as_ptr(),
                headers.len().saturating_sub(1) as u32,
                null(),
                0,
                0,
                0,
            )
        })?;
        request.deadline(started, timeout)?;
        check(unsafe { WinHttpReceiveResponse(request.0, null_mut()) })?;
        let mut status = 0_u32;
        let mut status_bytes = std::mem::size_of::<u32>() as u32;
        check(unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut status as *mut u32).cast(),
                &mut status_bytes,
                null_mut(),
            )
        })?;
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let location = request
                .header(WINHTTP_QUERY_LOCATION)?
                .ok_or_else(|| "La redirection de téléchargement est invalide.".to_string())?;
            current = redirect_url(&current, &location, followed)?;
            continue;
        }
        if status != 200 {
            return Err(format!("Le téléchargement a échoué (HTTP {status})."));
        }
        let expected = request
            .header(WINHTTP_QUERY_CONTENT_LENGTH)?
            .map(|length| {
                length
                    .parse::<usize>()
                    .ok()
                    .filter(|length| *length <= maximum)
                    .ok_or_else(|| "La taille du téléchargement est invalide.".to_string())
            })
            .transpose()?;
        let mut size = 0;
        let mut buffer = [0_u8; 8192];
        loop {
            request.deadline(started, timeout)?;
            let mut received = 0_u32;
            check(unsafe {
                WinHttpReadData(
                    request.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut received,
                )
            })?;
            if received == 0 {
                return if expected.is_none_or(|expected| expected == size) {
                    Ok(())
                } else {
                    Err("Le téléchargement est incomplet.".into())
                };
            }
            size = checked_size(size, received as usize, maximum)?;
            sink.write(&buffer[..received as usize])?;
        }
    }
    Err("Le téléchargement contient trop de redirections.".into())
}

#[cfg(not(windows))]
fn transfer(
    _url: &str,
    _maximum: usize,
    _sink: &mut Sink<'_>,
    _timeout: Duration,
) -> Result<(), String> {
    Err("Le téléchargement nécessite Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn accepts_only_fixed_github_https_authorities() {
        for host in [
            "api.github.com",
            "github.com",
            "release-assets.githubusercontent.com",
            "objects.githubusercontent.com",
            "raw.githubusercontent.com",
        ] {
            let address = format!("https://{host}/a%20b?token=A%2FB");
            let parsed = parse_url(&address).unwrap();
            assert_eq!(parsed.host, host);
            assert_eq!(parsed.path, "/a%20b?token=A%2FB");
        }
        assert_eq!(parse_url("https://GitHub.com").unwrap().path, "/");
    }

    #[test]
    fn rejects_ambiguous_authorities_and_request_injection() {
        for url in [
            "http://github.com/a",
            "https://github.com.evil.test/a",
            "https://evil.test/github.com/a",
            "https://user@github.com/a",
            "https://github.com:443/a",
            "https://github.com:444/a",
            "https://github.com./a",
            "https://%67ithub.com/a",
            "https://github.com\\evil.test/a",
            "https://github.com/a\\b",
            "https://github.com/a#fragment",
            "https://github.com/a b",
            "https://github.com/a\r\nInjected: yes",
            "https://github.com/a\0b",
            "https://github.com/a\u{7f}",
            "https://github.com/é",
            "https://github.com?query=x",
            "",
        ] {
            assert!(parse_url(url).is_err(), "{url:?}");
        }
        assert!(parse_url(&format!("https://github.com/{}", "a".repeat(MAX_URL_BYTES))).is_err());
    }

    #[test]
    fn validates_every_redirect_and_limits_hops() {
        let initial = "https://github.com/owner/project/releases/download/v1/app.exe";
        assert_eq!(
            redirect_url(initial, "/new/path", 0).unwrap(),
            "https://github.com/new/path"
        );
        let asset = "https://release-assets.githubusercontent.com/file?signature=a%2Fb";
        assert_eq!(redirect_url(initial, asset, 2).unwrap(), asset);
        assert!(redirect_url(initial, asset, 3).is_err());
        for target in [
            "http://github.com/file",
            "https://evil.test/file",
            "//evil.test/file",
            "//github.com/file",
            "../file",
            "/file\r\nHeader: value",
            "/file\\bad",
        ] {
            assert!(redirect_url(initial, target, 0).is_err(), "{target:?}");
        }
    }

    #[test]
    fn size_bound_handles_exact_limit_and_overflow() {
        assert_eq!(checked_size(8, 2, 10).unwrap(), 10);
        assert_eq!(checked_size(0, 0, 0).unwrap(), 0);
        assert!(checked_size(8, 3, 10).is_err());
        assert!(checked_size(usize::MAX, 1, usize::MAX).is_err());
    }

    fn temporary_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "SuperMenu.Http.Tests.{}.{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn failed_file_download_removes_partial_after_closing_handle() {
        let path = temporary_path();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        Sink::File(&mut file).write(b"partial response").unwrap();
        assert!(finish_file_download(file, &path, Err("interrupted".into())).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn completed_file_download_is_synced_and_existing_files_are_preserved() {
        let path = temporary_path();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        Sink::File(&mut file).write(b"complete response").unwrap();
        finish_file_download(file, &path, Ok(())).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"complete response");
        assert!(download("https://github.com/file", &path, 1024).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"complete response");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalid_address_never_creates_a_staging_file() {
        let path = temporary_path();
        assert!(download("http://github.com/file", &path, 1024).is_err());
        assert!(!path.exists());
    }
}

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderApi {
    OpenAiCompletions,
    OpenAiResponses,
    AnthropicMessages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderError {
    EmptyUrl,
    WhitespaceNotAllowed,
    ControlCharNotAllowed,
    InvalidScheme,
    HttpsRequired,
    EmbeddedCredentials,
    QueryNotAllowed,
    FragmentNotAllowed,
    EmptyHost,
    InvalidHost,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::EmptyUrl => write!(f, "provider URL cannot be empty"),
            ProviderError::WhitespaceNotAllowed => {
                write!(f, "provider URL cannot contain whitespace")
            }
            ProviderError::ControlCharNotAllowed => {
                write!(f, "provider URL cannot contain control characters")
            }
            ProviderError::InvalidScheme => {
                write!(f, "provider URL must use an http:// or https:// scheme")
            }
            ProviderError::HttpsRequired => {
                write!(
                    f,
                    "URL must use https:// (http:// is only allowed for loopback hosts)"
                )
            }
            ProviderError::EmbeddedCredentials => {
                write!(f, "provider URL cannot embed '@' credentials")
            }
            ProviderError::QueryNotAllowed => write!(f, "provider URL cannot contain '?' query"),
            ProviderError::FragmentNotAllowed => {
                write!(f, "provider URL cannot contain '#' fragment")
            }
            ProviderError::EmptyHost => write!(f, "provider URL must include a non-empty host"),
            ProviderError::InvalidHost => {
                write!(f, "provider URL contains an invalid host or port")
            }
        }
    }
}

impl std::error::Error for ProviderError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderUrl(String);

impl ProviderUrl {
    pub fn parse(raw: &str) -> Result<Self, ProviderError> {
        validate_provider_url(raw)?;
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalSignInUrl(String);

impl ExternalSignInUrl {
    pub fn parse(raw: &str) -> Result<Self, ProviderError> {
        validate_external_sign_in_url(raw)?;
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProfile {
    pub api: ProviderApi,
    pub base_url: ProviderUrl,
}

fn starts_with_bytes(haystack: &[u8], prefix: &[u8]) -> bool {
    if haystack.len() < prefix.len() {
        return false;
    }
    let mut i = 0usize;
    while i < prefix.len() {
        if haystack[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn eq_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0usize;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn eq_ignore_ascii_case_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0usize;
    while i < a.len() {
        if !a[i].eq_ignore_ascii_case(&b[i]) {
            return false;
        }
        i += 1;
    }
    true
}

fn is_ascii_whitespace_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn is_valid_port_bytes(port: &[u8]) -> bool {
    if port.is_empty() || port.len() > 5 {
        return false;
    }
    if port.len() > 1 && port[0] == b'0' {
        return false;
    }
    let mut value = 0u32;
    let mut p = 0usize;
    while p < port.len() {
        let b = port[p];
        if !b.is_ascii_digit() {
            return false;
        }
        value = value * 10 + (b - b'0') as u32;
        p += 1;
    }
    (1..=65535).contains(&value)
}

fn is_valid_reg_host_bytes(host: &[u8]) -> bool {
    if host.is_empty() {
        return false;
    }
    let first = host[0];
    let last = host[host.len() - 1];
    if first == b'.' || first == b'-' || last == b'.' || last == b'-' {
        return false;
    }
    let mut prev_dot = false;
    let mut i = 0usize;
    while i < host.len() {
        let b = host[i];
        let ok = b.is_ascii_alphanumeric() || b == b'.' || b == b'-';
        if !ok {
            return false;
        }
        if b == b'.' {
            if prev_dot {
                return false;
            }
            prev_dot = true;
        } else {
            prev_dot = false;
        }
        i += 1;
    }
    true
}

fn is_valid_ipv6_literal_inner(inner: &[u8]) -> bool {
    if inner.is_empty() {
        return false;
    }
    let mut has_colon = false;
    let mut i = 0usize;
    while i < inner.len() {
        let b = inner[i];
        if b == b':' {
            has_colon = true;
        } else if !(b.is_ascii_hexdigit() || b == b'.') {
            return false;
        }
        i += 1;
    }
    has_colon
}

fn is_ipv4_loopback_bytes(host: &[u8]) -> bool {
    if host.len() < 9 || host.len() > 15 || !starts_with_bytes(host, b"127.") {
        return false;
    }
    let mut idx = 4usize;
    let mut octets = 1usize;
    while octets < 4 {
        let start = idx;
        let mut val = 0u32;
        let mut digits = 0usize;
        while idx < host.len() && host[idx] != b'.' {
            let b = host[idx];
            if !b.is_ascii_digit() {
                return false;
            }
            digits += 1;
            if digits > 3 {
                return false;
            }
            val = val * 10 + (b - b'0') as u32;
            idx += 1;
        }
        if digits == 0 || (digits > 1 && host[start] == b'0') || val > 255 {
            return false;
        }
        octets += 1;
        if octets < 4 {
            if idx >= host.len() || host[idx] != b'.' {
                return false;
            }
            idx += 1;
        }
    }
    idx == host.len()
}

/// Allocation-free loopback host predicate verified directly by Kani.
/// Matches `localhost` (ASCII case-insensitive), IPv4 `127.0.0.0/8`, and IPv6 `[::1]`.
pub fn is_loopback_host_bytes(host: &[u8]) -> bool {
    eq_ignore_ascii_case_bytes(host, b"localhost")
        || eq_bytes(host, b"[::1]")
        || is_ipv4_loopback_bytes(host)
}

/// Allocation-free byte-level URL validator verified directly by Kani.
/// Rejects empty URLs, whitespace/control chars, non-http(s) schemes (or non-https
/// when `require_https` is true), non-loopback `http://`, embedded `@` credentials,
/// `?` query, `#` fragment, and empty/malformed hosts or ports.
pub fn validate_provider_url_bytes(bytes: &[u8], require_https: bool) -> Result<(), ProviderError> {
    if bytes.is_empty() {
        return Err(ProviderError::EmptyUrl);
    }

    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if is_ascii_whitespace_byte(b) {
            return Err(ProviderError::WhitespaceNotAllowed);
        }
        if b < 0x20 || b == 0x7f {
            return Err(ProviderError::ControlCharNotAllowed);
        }
        i += 1;
    }

    let is_https = starts_with_bytes(bytes, b"https://");
    let is_http = starts_with_bytes(bytes, b"http://");
    let scheme_len = if require_https {
        if is_https {
            8
        } else if is_http {
            return Err(ProviderError::HttpsRequired);
        } else {
            return Err(ProviderError::InvalidScheme);
        }
    } else if is_https {
        8
    } else if is_http {
        7
    } else {
        return Err(ProviderError::InvalidScheme);
    };

    let rest = &bytes[scheme_len..];
    let mut authority_end = rest.len();
    let mut found_slash = false;
    let mut j = 0usize;
    while j < rest.len() {
        let b = rest[j];
        if b == b'@' {
            return Err(ProviderError::EmbeddedCredentials);
        }
        if b == b'?' {
            return Err(ProviderError::QueryNotAllowed);
        }
        if b == b'#' {
            return Err(ProviderError::FragmentNotAllowed);
        }
        if b == b'/' && !found_slash {
            authority_end = j;
            found_slash = true;
        }
        j += 1;
    }

    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return Err(ProviderError::EmptyHost);
    }

    let host = if authority[0] == b'[' {
        let mut close_idx = None;
        let mut k = 1usize;
        while k < authority.len() {
            if authority[k] == b']' {
                close_idx = Some(k);
                break;
            }
            k += 1;
        }
        let Some(end_bracket) = close_idx else {
            return Err(ProviderError::InvalidHost);
        };
        if end_bracket <= 1 {
            return Err(ProviderError::EmptyHost);
        }
        if !is_valid_ipv6_literal_inner(&authority[1..end_bracket]) {
            return Err(ProviderError::InvalidHost);
        }
        let after = &authority[end_bracket + 1..];
        if !after.is_empty() && (after[0] != b':' || !is_valid_port_bytes(&after[1..])) {
            return Err(ProviderError::InvalidHost);
        }
        &authority[..=end_bracket]
    } else {
        let mut colon_idx = None;
        let mut k = 0usize;
        while k < authority.len() {
            if authority[k] == b':' {
                colon_idx = Some(k);
                break;
            }
            k += 1;
        }
        let host_slice = match colon_idx {
            None => authority,
            Some(0) => return Err(ProviderError::EmptyHost),
            Some(idx) => {
                let port = &authority[idx + 1..];
                if !is_valid_port_bytes(port) {
                    return Err(ProviderError::InvalidHost);
                }
                &authority[..idx]
            }
        };
        if !is_valid_reg_host_bytes(host_slice) {
            return Err(ProviderError::InvalidHost);
        }
        host_slice
    };

    if is_http && !is_loopback_host_bytes(host) {
        return Err(ProviderError::HttpsRequired);
    }

    Ok(())
}

/// Allocation-free byte-level external sign-in URL validator (`https://` strictly).
pub fn validate_external_sign_in_url_bytes(bytes: &[u8]) -> Result<(), ProviderError> {
    validate_provider_url_bytes(bytes, true)
}

/// Validates a custom provider base URL (`https://`, or `http://` on loopback hosts only).
pub fn validate_provider_url(url: &str) -> Result<(), ProviderError> {
    validate_provider_url_bytes(url.as_bytes(), false)
}

/// Validates an external sign-in URL (`https://` strictly).
pub fn validate_external_sign_in_url(url: &str) -> Result<(), ProviderError> {
    validate_external_sign_in_url_bytes(url.as_bytes())
}

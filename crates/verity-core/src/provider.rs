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
            ProviderError::HttpsRequired => write!(f, "external sign-in URL must use https://"),
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

fn is_ascii_whitespace_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Allocation-free byte-level URL validator verified directly by Kani.
/// Rejects empty URLs, whitespace/control chars, non-http(s) schemes (or non-https
/// when `require_https` is true), embedded `@` credentials, `?` query, `#` fragment,
/// and empty/malformed hosts.
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

    if authority[0] == b'[' {
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
        let after = &authority[end_bracket + 1..];
        if !after.is_empty() {
            if after[0] != b':' || after.len() == 1 || after.len() > 6 {
                return Err(ProviderError::InvalidHost);
            }
            let mut p = 1usize;
            while p < after.len() {
                if !after[p].is_ascii_digit() {
                    return Err(ProviderError::InvalidHost);
                }
                p += 1;
            }
        }
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
        match colon_idx {
            None => {}
            Some(0) => return Err(ProviderError::EmptyHost),
            Some(idx) => {
                let port = &authority[idx + 1..];
                if port.is_empty() || port.len() > 5 {
                    return Err(ProviderError::InvalidHost);
                }
                let mut p = 0usize;
                while p < port.len() {
                    if !port[p].is_ascii_digit() {
                        return Err(ProviderError::InvalidHost);
                    }
                    p += 1;
                }
            }
        }
    }

    Ok(())
}

/// Validates a custom provider base URL (`http://` or `https://`).
pub fn validate_provider_url(url: &str) -> Result<(), ProviderError> {
    validate_provider_url_bytes(url.as_bytes(), false)
}

/// Validates an external sign-in URL (`https://` strictly).
pub fn validate_external_sign_in_url(url: &str) -> Result<(), ProviderError> {
    validate_provider_url_bytes(url.as_bytes(), true)
}

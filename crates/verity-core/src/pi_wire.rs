//! Pi message tags parsed by the RPC client.

/// Message role parsed at the Pi RPC wire boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireRole {
    User,
    Assistant,
    ToolResult,
    Other,
}

impl WireRole {
    /// Parses a wire role string into a typed `WireRole` at the RPC boundary.
    pub fn parse_str(raw: &str) -> Self {
        match raw {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "toolResult" | "tool_result" | "tool" => Self::ToolResult,
            _ => Self::Other,
        }
    }
}

/// Assistant message stop reason parsed at the Pi RPC wire boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireStopReason {
    Stop,
    Length,
    ToolUse,
    Aborted,
    Error,
    Unknown,
}

impl WireStopReason {
    /// Parses a wire stop-reason string into a typed `WireStopReason` at the RPC boundary.
    pub fn parse_str(raw: &str) -> Self {
        match raw {
            "stop" | "end_turn" => Self::Stop,
            "length" | "max_tokens" => Self::Length,
            "toolUse" | "tool_use" | "tool_calls" => Self::ToolUse,
            "aborted" | "cancelled" | "canceled" => Self::Aborted,
            "error" => Self::Error,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_parsers_handle_known_and_unknown_wire_strings() {
        assert_eq!(WireRole::parse_str("user"), WireRole::User);
        assert_eq!(WireRole::parse_str("assistant"), WireRole::Assistant);
        assert_eq!(WireRole::parse_str("toolResult"), WireRole::ToolResult);
        assert_eq!(WireRole::parse_str("system"), WireRole::Other);

        assert_eq!(WireStopReason::parse_str("stop"), WireStopReason::Stop);
        assert_eq!(
            WireStopReason::parse_str("tool_use"),
            WireStopReason::ToolUse
        );
        assert_eq!(WireStopReason::parse_str("error"), WireStopReason::Error);
        assert_eq!(
            WireStopReason::parse_str("aborted"),
            WireStopReason::Aborted
        );
        assert_eq!(WireStopReason::parse_str("other"), WireStopReason::Unknown);
    }
}

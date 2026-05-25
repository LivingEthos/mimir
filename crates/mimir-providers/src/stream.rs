//! Server-Sent Events (SSE) streaming support for provider responses.
//!
//! Parses `text/event-stream` chunks into structured stream events.

use serde::Deserialize;

/// A single SSE event parsed from the stream.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// Content block delta (text, tool_use, etc.).
    ContentBlockDelta {
        /// Index of the content block.
        index: usize,
        /// The delta payload.
        delta: ContentDelta,
    },
    /// Content block start.
    ContentBlockStart {
        /// Index of the content block.
        index: usize,
        /// Block type and initial content.
        content_block: ContentBlock,
    },
    /// Content block stop.
    ContentBlockStop {
        /// Index of the content block.
        index: usize,
    },
    /// Message start event.
    MessageStart {
        /// Message metadata.
        message: MessageMeta,
    },
    /// Message delta (usage, stop_reason updates).
    MessageDelta {
        /// Delta payload.
        delta: MessageDeltaPayload,
    },
    /// Message stop event.
    MessageStop,
    /// Ping event (keepalive).
    Ping,
    /// Unknown event type (forwarded raw for debugging).
    Unknown {
        /// Event type string.
        event_type: String,
        /// Raw JSON data.
        data: String,
    },
}

/// Content delta variants.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentDelta {
    /// Text delta.
    TextDelta {
        /// Text fragment.
        text: String,
    },
    /// Tool use delta (partial JSON).
    ToolUseDelta {
        /// Partial JSON input.
        partial_json: String,
    },
    /// Thinking delta (reasoning content).
    ThinkingDelta {
        /// Thinking text.
        thinking: String,
    },
    /// Signature delta (for thinking blocks).
    SignatureDelta {
        /// Signature string.
        signature: String,
    },
}

/// A content block in a stream.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ContentBlock {
    /// Block type: text, tool_use, thinking.
    #[serde(rename = "type")]
    pub block_type: String,
    /// Text content (for text blocks).
    pub text: Option<String>,
    /// Tool use ID.
    pub id: Option<String>,
    /// Tool name.
    pub name: Option<String>,
    /// Tool input JSON.
    pub input: Option<serde_json::Value>,
    /// Thinking content.
    pub thinking: Option<String>,
    /// Signature for thinking blocks.
    pub signature: Option<String>,
}

/// Message metadata from stream start.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MessageMeta {
    /// Message ID.
    pub id: String,
    /// Message type.
    #[serde(rename = "type")]
    pub message_type: String,
    /// Role.
    pub role: String,
    /// Model name.
    pub model: String,
    /// Stop reason (if already known).
    pub stop_reason: Option<String>,
    /// Stop sequence (if any).
    pub stop_sequence: Option<String>,
    /// Usage at start (usually empty).
    pub usage: Option<serde_json::Value>,
}

/// Message delta payload.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct MessageDeltaPayload {
    /// Stop reason update.
    pub stop_reason: Option<String>,
    /// Stop sequence update.
    pub stop_sequence: Option<String>,
    /// Usage update.
    pub usage: Option<StreamUsage>,
}

/// Usage data from streaming.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct StreamUsage {
    /// Input tokens.
    pub input_tokens: Option<u32>,
    /// Output tokens.
    pub output_tokens: Option<u32>,
    /// Cache creation input tokens.
    pub cache_creation_input_tokens: Option<u32>,
    /// Cache read input tokens.
    pub cache_read_input_tokens: Option<u32>,
}

/// SSE parser state machine.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    /// Create a new SSE parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw bytes into the parser and return completed events.
    pub fn feed(&mut self, chunk: &str) -> Vec<StreamEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();

        // Process complete SSE events (double newline separated)
        while let Some(pos) = self.buffer.find("\n\n") {
            let raw = self.buffer[..pos].to_string();
            self.buffer.drain(..pos + 2);

            if let Some(event) = Self::parse_event(&raw) {
                events.push(event);
            }
        }

        events
    }

    /// Flush any remaining buffered data (best-effort).
    pub fn flush(&mut self) -> Vec<StreamEvent> {
        let remaining = self.buffer.trim();
        let mut events = Vec::new();
        if !remaining.is_empty() {
            if let Some(event) = Self::parse_event(remaining) {
                events.push(event);
            }
        }
        self.buffer.clear();
        events
    }

    fn parse_event(raw: &str) -> Option<StreamEvent> {
        let mut event_type = "message";
        let mut data_lines = Vec::new();

        for line in raw.lines() {
            if let Some(stripped) = line.strip_prefix("event:") {
                event_type = stripped.trim();
            } else if let Some(stripped) = line.strip_prefix("data:") {
                data_lines.push(stripped.trim());
            } else if line.starts_with("id:") || line.starts_with(":") {
                // Ignore ID lines and comments
                continue;
            }
        }

        let data = data_lines.join("");

        match event_type {
            "ping" => Some(StreamEvent::Ping),
            "message_start" => serde_json::from_str::<MessageStartPayload>(&data)
                .ok()
                .map(|p| StreamEvent::MessageStart { message: p.message }),
            "content_block_start" => serde_json::from_str::<ContentBlockStartPayload>(&data)
                .ok()
                .map(|p| StreamEvent::ContentBlockStart {
                    index: p.index,
                    content_block: p.content_block,
                }),
            "content_block_delta" => serde_json::from_str::<ContentBlockDeltaPayload>(&data)
                .ok()
                .and_then(|p| {
                    Self::parse_delta(p.delta).map(|delta| StreamEvent::ContentBlockDelta {
                        index: p.index,
                        delta,
                    })
                }),
            "content_block_stop" => serde_json::from_str::<ContentBlockStopPayload>(&data)
                .ok()
                .map(|p| StreamEvent::ContentBlockStop { index: p.index }),
            "message_delta" => serde_json::from_str::<MessageDeltaPayload>(&data)
                .ok()
                .map(|delta| StreamEvent::MessageDelta { delta }),
            "message_stop" => Some(StreamEvent::MessageStop),
            _ => Some(StreamEvent::Unknown {
                event_type: event_type.to_string(),
                data,
            }),
        }
    }

    fn parse_delta(delta: serde_json::Value) -> Option<ContentDelta> {
        let delta_type = delta.get("type")?.as_str()?;
        match delta_type {
            "text_delta" => {
                delta
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|text| ContentDelta::TextDelta {
                        text: text.to_string(),
                    })
            }
            "input_json_delta" => delta
                .get("partial_json")
                .and_then(|v| v.as_str())
                .map(|json| ContentDelta::ToolUseDelta {
                    partial_json: json.to_string(),
                }),
            "thinking_delta" => delta
                .get("thinking")
                .and_then(|v| v.as_str())
                .map(|thinking| ContentDelta::ThinkingDelta {
                    thinking: thinking.to_string(),
                }),
            "signature_delta" => delta.get("signature").and_then(|v| v.as_str()).map(|sig| {
                ContentDelta::SignatureDelta {
                    signature: sig.to_string(),
                }
            }),
            _ => None,
        }
    }
}

// Helper structs for deserializing SSE payloads
#[derive(Deserialize)]
struct MessageStartPayload {
    message: MessageMeta,
}

#[derive(Deserialize)]
struct ContentBlockStartPayload {
    index: usize,
    content_block: ContentBlock,
}

#[derive(Deserialize)]
struct ContentBlockDeltaPayload {
    index: usize,
    delta: serde_json::Value,
}

#[derive(Deserialize)]
struct ContentBlockStopPayload {
    index: usize,
}

/// Accumulate stream events into a final response.
#[derive(Debug, Default)]
pub struct StreamAccumulator {
    /// Accumulated text content.
    pub text: String,
    /// Accumulated tool uses.
    pub tool_uses: Vec<ToolUseAccumulator>,
    /// Current tool use being accumulated.
    current_tool: Option<ToolUseAccumulator>,
    /// Message metadata.
    pub message_meta: Option<MessageMeta>,
    /// Final usage.
    pub usage: StreamUsage,
    /// Final stop reason.
    pub stop_reason: Option<String>,
}

/// Accumulator for a single tool use.
#[derive(Debug, Clone)]
pub struct ToolUseAccumulator {
    /// Tool use ID.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Accumulated partial JSON.
    pub partial_json: String,
}

impl StreamAccumulator {
    /// Create a new accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a stream event.
    pub fn push(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::MessageStart { message } => {
                self.message_meta = Some(message.clone());
            }
            StreamEvent::ContentBlockStart { content_block, .. }
                if content_block.block_type == "tool_use" =>
            {
                self.current_tool = Some(ToolUseAccumulator {
                    id: content_block.id.clone().unwrap_or_default(),
                    name: content_block.name.clone().unwrap_or_default(),
                    partial_json: String::new(),
                });
            }
            StreamEvent::ContentBlockDelta { delta, .. } => match delta {
                ContentDelta::TextDelta { text } => {
                    self.text.push_str(text);
                }
                ContentDelta::ToolUseDelta { partial_json } => {
                    if let Some(ref mut tool) = self.current_tool {
                        tool.partial_json.push_str(partial_json);
                    }
                }
                _ => {}
            },
            StreamEvent::ContentBlockStop { .. } => {
                if let Some(tool) = self.current_tool.take() {
                    self.tool_uses.push(tool);
                }
            }
            StreamEvent::MessageDelta { delta } => {
                if let Some(reason) = &delta.stop_reason {
                    self.stop_reason = Some(reason.clone());
                }
                if let Some(usage) = &delta.usage {
                    self.usage = usage.clone();
                }
            }
            _ => {}
        }
    }

    /// Convert accumulated tool uses into parsed ResponseBlocks.
    pub fn parsed_tool_uses(&self) -> Vec<crate::types::ResponseBlock> {
        self.tool_uses
            .iter()
            .map(|tool| {
                let input = serde_json::from_str(&tool.partial_json).unwrap_or_default();
                crate::types::ResponseBlock::ToolUse {
                    id: tool.id.clone(),
                    name: tool.name.clone(),
                    input,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ping() {
        let mut parser = SseParser::new();
        let events = parser.feed("event: ping\n\n");
        assert_eq!(events, vec![StreamEvent::Ping]);
    }

    #[test]
    fn test_parse_message_stop() {
        let mut parser = SseParser::new();
        let events = parser.feed("event: message_stop\ndata: {}\n\n");
        assert_eq!(events, vec![StreamEvent::MessageStop]);
    }

    #[test]
    fn test_parse_text_delta() {
        let mut parser = SseParser::new();
        let events = parser.feed(
            "event: content_block_delta\n\
             data: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            StreamEvent::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta { text },
            } if text == "Hello"
        ));
    }

    #[test]
    fn test_accumulator_text() {
        let mut acc = StreamAccumulator::new();
        acc.push(&StreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta {
                text: "Hello ".into(),
            },
        });
        acc.push(&StreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::TextDelta {
                text: "world".into(),
            },
        });
        assert_eq!(acc.text, "Hello world");
    }

    #[test]
    fn test_accumulator_tool_use() {
        let mut acc = StreamAccumulator::new();
        acc.push(&StreamEvent::ContentBlockStart {
            index: 0,
            content_block: ContentBlock {
                block_type: "tool_use".into(),
                text: None,
                id: Some("tool_1".into()),
                name: Some("get_weather".into()),
                input: None,
                thinking: None,
                signature: None,
            },
        });
        acc.push(&StreamEvent::ContentBlockDelta {
            index: 0,
            delta: ContentDelta::ToolUseDelta {
                partial_json: "{\"city\":\"Paris\"}".into(),
            },
        });
        acc.push(&StreamEvent::ContentBlockStop { index: 0 });
        assert_eq!(acc.tool_uses.len(), 1);
        assert_eq!(acc.tool_uses[0].name, "get_weather");
        assert_eq!(acc.tool_uses[0].partial_json, "{\"city\":\"Paris\"}");
    }

    #[test]
    fn test_parse_unknown_event() {
        let mut parser = SseParser::new();
        let events = parser.feed("event: custom_event\ndata: {\"foo\":\"bar\"}\n\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            StreamEvent::Unknown { event_type, .. } if event_type == "custom_event"
        ));
    }
}

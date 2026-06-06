use std::{env, time::Duration};

use anyhow::{bail, Context, Result};
use duihua_common::response_store_from_env;
use redis::{
    aio::ConnectionManager,
    streams::{
        StreamAutoClaimOptions, StreamAutoClaimReply, StreamDeletionPolicy, StreamId,
        StreamReadOptions,
    },
    AsyncCommands, RedisError,
};

use crate::worker::{self, ProcessContext, ProcessOutcome};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueMessage {
    pub stream_id: String,
    pub response_id: String,
    pub idle_ms: Option<u64>,
}

pub struct QueueConfig {
    pub redis_url: String,
    pub stream_key: String,
    pub consumer_group: String,
    pub consumer_name: String,
    pub block_ms: usize,
    pub autoclaim_min_idle_ms: usize,
    pub autoclaim_batch_size: usize,
}

impl QueueConfig {
    pub fn from_env() -> Result<Self> {
        let redis_url =
            env::var("RESPONSE_ID_STORE_URL").unwrap_or_else(|_| "redis://valkey:6379".to_string());
        let stream_key = env::var("BACKGROUND_QUEUE_STREAM_KEY")
            .unwrap_or_else(|_| "duihua:responses:background".to_string());
        let consumer_group = env::var("BACKGROUND_QUEUE_CONSUMER_GROUP")
            .unwrap_or_else(|_| "duihua-background".to_string());
        let consumer_name = env::var("BACKGROUND_QUEUE_CONSUMER_NAME")
            .or_else(|_| env::var("HOSTNAME"))
            .unwrap_or_else(|_| "duihua-background-worker".to_string());
        let block_ms = env::var("BACKGROUND_QUEUE_BLOCK_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5_000);
        if block_ms == 0 {
            bail!("BACKGROUND_QUEUE_BLOCK_MS must be greater than 0");
        }
        let autoclaim_min_idle_ms = env::var("BACKGROUND_QUEUE_AUTOCLAIM_MIN_IDLE_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(default_autoclaim_min_idle_ms);
        warn_if_autoclaim_shorter_than_upstream(autoclaim_min_idle_ms);
        let autoclaim_batch_size = env::var("BACKGROUND_QUEUE_AUTOCLAIM_BATCH_SIZE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(10);

        Ok(Self {
            redis_url,
            stream_key,
            consumer_group,
            consumer_name,
            block_ms,
            autoclaim_min_idle_ms,
            autoclaim_batch_size,
        })
    }
}

pub async fn run() -> Result<()> {
    let config = QueueConfig::from_env()?;
    let response_store = response_store_from_env().await?;
    let client = redis::Client::open(config.redis_url.as_str())
        .with_context(|| format!("invalid RESPONSE_ID_STORE_URL {}", config.redis_url))?;
    let mut connection = ConnectionManager::new(client)
        .await
        .with_context(|| "failed to connect to background queue")?;

    ensure_consumer_group(&mut connection, &config).await?;
    drain_pending_at_startup(&mut connection, &config, &response_store).await;

    let mut autoclaim_cursor = "0-0".to_string();

    loop {
        let autoclaim_result: Result<StreamAutoClaimReply, RedisError> = connection
            .xautoclaim_options(
                &config.stream_key,
                &config.consumer_group,
                &config.consumer_name,
                config.autoclaim_min_idle_ms,
                &autoclaim_cursor,
                StreamAutoClaimOptions::default().count(config.autoclaim_batch_size),
            )
            .await;
        match autoclaim_result {
            Ok(autoclaim) => {
                autoclaim_cursor = autoclaim.next_stream_id;
                process_stream_entries(
                    &mut connection,
                    &config,
                    &response_store,
                    &autoclaim.claimed,
                    false,
                )
                .await;
            }
            Err(err) => {
                eprintln!("failed to auto-claim background queue messages: {err:?}");
                sleep_on_redis_error().await;
                continue;
            }
        }

        let new_opts = StreamReadOptions::default()
            .group(&config.consumer_group, &config.consumer_name)
            .block(config.block_ms)
            .count(1);
        let read_result: Result<Option<redis::streams::StreamReadReply>, RedisError> = connection
            .xread_options(&[&config.stream_key], &[">"], &new_opts)
            .await;
        match read_result {
            Ok(Some(reply)) => {
                let entries: Vec<StreamId> = reply
                    .keys
                    .iter()
                    .flat_map(|key| key.ids.iter().cloned())
                    .collect();
                process_stream_entries(&mut connection, &config, &response_store, &entries, false)
                    .await;
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("failed to read new background queue messages: {err:?}");
                sleep_on_redis_error().await;
            }
        }
    }
}

async fn drain_pending_at_startup(
    connection: &mut ConnectionManager,
    config: &QueueConfig,
    response_store: &duihua_common::ResponseStore,
) {
    let pending_opts = StreamReadOptions::default()
        .group(&config.consumer_group, &config.consumer_name)
        .count(config.autoclaim_batch_size);

    loop {
        let reply: Option<redis::streams::StreamReadReply> = match connection
            .xread_options(&[&config.stream_key], &["0"], &pending_opts)
            .await
        {
            Ok(reply) => reply,
            Err(err) => {
                eprintln!("failed to drain pending background queue messages at startup: {err:?}");
                sleep_on_redis_error().await;
                break;
            }
        };

        let Some(reply) = reply else {
            break;
        };

        let entries: Vec<StreamId> = reply
            .keys
            .iter()
            .flat_map(|key| key.ids.iter().cloned())
            .collect();
        if entries.is_empty() {
            break;
        }

        if process_stream_entries(connection, config, response_store, &entries, true)
            .await
            .stopped_on_error
        {
            break;
        }
    }
}

struct ProcessBatchOutcome {
    stopped_on_error: bool,
}

async fn process_stream_entries(
    connection: &mut ConnectionManager,
    config: &QueueConfig,
    response_store: &duihua_common::ResponseStore,
    entries: &[StreamId],
    stop_on_error: bool,
) -> ProcessBatchOutcome {
    let (messages, invalid_ids) = split_stream_entries(entries);
    let mut stopped_on_error = false;

    for stream_id in invalid_ids {
        eprintln!("acknowledging malformed background queue entry {stream_id}");
        if let Err(err) = acknowledge_message(connection, config, &stream_id).await {
            eprintln!("failed to acknowledge malformed entry {stream_id}: {err:?}");
            if stop_on_error {
                stopped_on_error = true;
                break;
            }
        }
    }

    if stopped_on_error {
        return ProcessBatchOutcome { stopped_on_error };
    }

    for message in messages {
        let response_id = message.response_id.clone();
        match handle_message(connection, config, response_store, message).await {
            Ok(()) => {}
            Err(err) => {
                eprintln!("failed to process background queue message {response_id}: {err:?}");
                if stop_on_error || err.downcast_ref::<RetryableMessageError>().is_some() {
                    stopped_on_error = true;
                    break;
                }
            }
        }
    }

    ProcessBatchOutcome { stopped_on_error }
}

async fn ensure_consumer_group(
    connection: &mut ConnectionManager,
    config: &QueueConfig,
) -> Result<()> {
    match connection
        .xgroup_create_mkstream(&config.stream_key, &config.consumer_group, "0")
        .await
    {
        Ok(()) => Ok(()),
        Err(err) if is_busygroup(&err) => Ok(()),
        Err(err) => Err(err).context("failed to create background queue consumer group"),
    }
}

#[derive(Debug)]
struct RetryableMessageError;

impl std::fmt::Display for RetryableMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("background queue message will be retried")
    }
}

impl std::error::Error for RetryableMessageError {}

async fn handle_message(
    connection: &mut ConnectionManager,
    config: &QueueConfig,
    response_store: &duihua_common::ResponseStore,
    message: QueueMessage,
) -> Result<()> {
    let ctx = ProcessContext {
        message_idle_ms: message.idle_ms,
        autoclaim_min_idle_ms: config.autoclaim_min_idle_ms,
    };
    match worker::process_response(response_store, &message.response_id, ctx).await? {
        ProcessOutcome::Ack => {
            acknowledge_message(connection, config, &message.stream_id).await?;
        }
        ProcessOutcome::Retry => {
            return Err(RetryableMessageError.into());
        }
    }
    Ok(())
}

async fn acknowledge_message(
    connection: &mut ConnectionManager,
    config: &QueueConfig,
    stream_id: &str,
) -> Result<()> {
    let ids = [stream_id];
    match connection
        .xack_del::<_, _, _, Vec<redis::streams::XAckDelStatusCode>>(
            &config.stream_key,
            &config.consumer_group,
            &ids,
            StreamDeletionPolicy::Acked,
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(err) if is_unsupported_xackdel(&err) => {
            let _: usize = connection
                .xack(&config.stream_key, &config.consumer_group, &ids)
                .await?;
            let _: usize = connection.xdel(&config.stream_key, &ids).await?;
            Ok(())
        }
        Err(err) => Err(err).context("failed to acknowledge background queue message"),
    }
}

fn default_autoclaim_min_idle_ms() -> usize {
    upstream_timeout_seconds_from_env()
        .saturating_add(120)
        .saturating_mul(1000)
}

fn upstream_timeout_seconds_from_env() -> usize {
    env::var("BACKGROUND_UPSTREAM_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600)
}

fn warn_if_autoclaim_shorter_than_upstream(autoclaim_min_idle_ms: usize) {
    let upstream_ms = upstream_timeout_seconds_from_env().saturating_mul(1000);
    if autoclaim_min_idle_ms < upstream_ms {
        eprintln!(
            "warning: BACKGROUND_QUEUE_AUTOCLAIM_MIN_IDLE_MS ({autoclaim_min_idle_ms}) is shorter than BACKGROUND_UPSTREAM_TIMEOUT_SECONDS ({upstream_ms} ms); active upstream calls may be reclaimed and marked failed"
        );
    }
}

async fn sleep_on_redis_error() {
    tokio::time::sleep(Duration::from_secs(1)).await;
}

pub fn response_id_from_stream_entry(entry: &StreamId) -> Option<String> {
    entry.get("response_id")
}

pub fn queue_message_from_stream_entry(entry: &StreamId) -> Option<QueueMessage> {
    response_id_from_stream_entry(entry).map(|response_id| QueueMessage {
        stream_id: entry.id.clone(),
        response_id,
        idle_ms: entry
            .milliseconds_elapsed_from_delivery
            .map(|idle| idle as u64),
    })
}

pub fn split_stream_entries(entries: &[StreamId]) -> (Vec<QueueMessage>, Vec<String>) {
    let mut messages = Vec::new();
    let mut invalid_ids = Vec::new();
    for entry in entries {
        if let Some(message) = queue_message_from_stream_entry(entry) {
            messages.push(message);
        } else {
            invalid_ids.push(entry.id.clone());
        }
    }
    (messages, invalid_ids)
}

fn is_busygroup(err: &RedisError) -> bool {
    err.code() == Some("BUSYGROUP")
}

fn is_unsupported_xackdel(err: &RedisError) -> bool {
    err.to_string()
        .to_ascii_lowercase()
        .contains("unknown command")
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis::FromRedisValue;
    use redis::Value;

    #[test]
    fn extracts_response_id_from_stream_entry() {
        let entry = StreamId {
            id: "1717670000000-0".to_string(),
            map: [(
                "response_id".to_string(),
                Value::BulkString(b"resp_abc".to_vec()),
            )]
            .into(),
            milliseconds_elapsed_from_delivery: Some(42_usize),
            delivered_count: None,
        };

        assert_eq!(
            response_id_from_stream_entry(&entry).as_deref(),
            Some("resp_abc")
        );
        assert_eq!(
            queue_message_from_stream_entry(&entry),
            Some(QueueMessage {
                stream_id: "1717670000000-0".to_string(),
                response_id: "resp_abc".to_string(),
                idle_ms: Some(42),
            })
        );
    }

    #[test]
    fn splits_invalid_stream_entries_for_explicit_ack() {
        let valid = StreamId {
            id: "1717670000000-0".to_string(),
            map: [(
                "response_id".to_string(),
                Value::BulkString(b"resp_abc".to_vec()),
            )]
            .into(),
            milliseconds_elapsed_from_delivery: None,
            delivered_count: None,
        };
        let invalid = StreamId {
            id: "1717670000001-0".to_string(),
            map: [("other".to_string(), Value::BulkString(b"x".to_vec()))].into(),
            milliseconds_elapsed_from_delivery: None,
            delivered_count: None,
        };

        let (messages, invalid_ids) = split_stream_entries(&[valid, invalid]);
        assert_eq!(messages.len(), 1);
        assert_eq!(invalid_ids, vec!["1717670000001-0".to_string()]);
    }

    #[test]
    fn parses_messages_from_xreadgroup_reply() {
        let value = Value::Array(vec![Value::Array(vec![
            Value::BulkString(b"duihua:responses:background".to_vec()),
            Value::Array(vec![Value::Array(vec![
                Value::BulkString(b"1717670000000-0".to_vec()),
                Value::Array(vec![
                    Value::BulkString(b"response_id".to_vec()),
                    Value::BulkString(b"resp_xyz".to_vec()),
                ]),
            ])]),
        ])]);
        let reply = redis::streams::StreamReadReply::from_redis_value(value).expect("reply");
        let entries: Vec<StreamId> = reply
            .keys
            .iter()
            .flat_map(|key| key.ids.iter().cloned())
            .collect();
        assert_eq!(
            split_stream_entries(&entries).0,
            vec![QueueMessage {
                stream_id: "1717670000000-0".to_string(),
                response_id: "resp_xyz".to_string(),
                idle_ms: None,
            }]
        );
    }

    #[test]
    fn rejects_zero_block_ms() {
        env::set_var("BACKGROUND_QUEUE_BLOCK_MS", "0");
        assert!(QueueConfig::from_env().is_err());
        env::remove_var("BACKGROUND_QUEUE_BLOCK_MS");
    }

    #[test]
    fn default_autoclaim_exceeds_upstream_timeout() {
        env::remove_var("BACKGROUND_UPSTREAM_TIMEOUT_SECONDS");
        env::remove_var("BACKGROUND_QUEUE_AUTOCLAIM_MIN_IDLE_MS");
        let config = QueueConfig::from_env().expect("config");
        assert_eq!(config.autoclaim_min_idle_ms, 720_000);
    }
}

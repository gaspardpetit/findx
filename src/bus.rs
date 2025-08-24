use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossbeam_channel::{bounded, Receiver, Sender};
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::BusBounds;
use crate::events::{MirrorEvent, SourceEvent};

#[derive(Debug, Clone, Serialize)]
pub struct Envelope<T> {
    pub v: u8,
    pub ts: i64,
    pub idempotency_key: String,
    pub data: T,
}

impl<T: Serialize> Envelope<T> {
    pub fn new(event: T) -> Self {
        Self {
            v: 1,
            ts: now(),
            idempotency_key: compute_idempotency_key(&event),
            data: event,
        }
    }
}

fn compute_idempotency_key<T: Serialize>(event: &T) -> String {
    let json = serde_json::to_vec(event).expect("serialize event");
    let hash = Sha256::digest(&json);
    format!("sha256:{:x}", hash)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[derive(Clone)]
struct Topic<T> {
    bound: usize,
    subs: Arc<Mutex<Vec<Sender<Envelope<T>>>>>,
}

impl<T> Topic<T>
where
    T: Serialize + Clone + Send + 'static,
{
    fn new(bound: usize) -> Self {
        Self {
            bound,
            subs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn publish(&self, env: Envelope<T>) {
        let mut subs = self.subs.lock().unwrap();
        subs.retain(|tx| tx.send(env.clone()).is_ok());
    }

    fn subscribe(&self) -> Receiver<Envelope<T>> {
        let (tx, rx) = bounded(self.bound);
        self.subs.lock().unwrap().push(tx);
        rx
    }
}

#[derive(Clone)]
pub struct EventBus {
    source: Topic<SourceEvent>,
    mirror: Topic<MirrorEvent>,
    conn: Arc<Mutex<Connection>>,
}

impl EventBus {
    pub fn new(bounds: &BusBounds, conn: Arc<Mutex<Connection>>) -> Self {
        Self {
            source: Topic::new(bounds.source_fs),
            mirror: Topic::new(bounds.mirror_text),
            conn,
        }
    }

    fn log_event<T: Serialize>(&self, topic: &str, env: &Envelope<T>) -> Result<()> {
        let payload = serde_json::to_string(&env.data)?;
        let event_type = serde_json::to_value(&env.data)?
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (ts, topic, type, idempotency_key, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![env.ts, topic, event_type, env.idempotency_key, payload],
        )?;
        Ok(())
    }

    pub fn publish_source(&self, event: SourceEvent) -> Result<()> {
        let env = Envelope::new(event);
        self.source.publish(env.clone());
        self.log_event("source.fs", &env)
    }

    pub fn subscribe_source(&self) -> Receiver<Envelope<SourceEvent>> {
        self.source.subscribe()
    }

    pub fn publish_mirror(&self, event: MirrorEvent) -> Result<()> {
        let env = Envelope::new(event);
        self.mirror.publish(env.clone());
        self.log_event("mirror.text", &env)
    }

    pub fn subscribe_mirror(&self) -> Receiver<Envelope<MirrorEvent>> {
        self.mirror.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8Path;

    #[test]
    fn publish_subscribe_roundtrip() {
        let conn = crate::db::open(Utf8Path::new(":memory:")).unwrap();
        let bus = EventBus::new(
            &BusBounds {
                source_fs: 10,
                mirror_text: 10,
            },
            Arc::new(Mutex::new(conn)),
        );

        let source_rx = bus.subscribe_source();
        bus.publish_source(SourceEvent::SyncStarted).unwrap();
        let env = source_rx.recv().unwrap();
        assert_eq!(env.data, SourceEvent::SyncStarted);
        assert_eq!(env.idempotency_key, compute_idempotency_key(&env.data));

        let mirror_rx = bus.subscribe_mirror();
        bus.publish_mirror(MirrorEvent::MirrorDocDeleted {
            file_uid: "f1".into(),
        })
        .unwrap();
        let env2 = mirror_rx.recv().unwrap();
        match env2.data {
            MirrorEvent::MirrorDocDeleted { ref file_uid } => assert_eq!(file_uid, "f1"),
            _ => panic!("wrong event"),
        }
        assert_eq!(env2.idempotency_key, compute_idempotency_key(&env2.data));

        let conn = bus.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT topic, type FROM events ORDER BY id")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        let row1 = rows.next().unwrap().unwrap();
        let topic1: String = row1.get(0).unwrap();
        let type1: String = row1.get(1).unwrap();
        assert_eq!(topic1, "source.fs");
        assert_eq!(type1, "sync_started");
        let row2 = rows.next().unwrap().unwrap();
        let topic2: String = row2.get(0).unwrap();
        let type2: String = row2.get(1).unwrap();
        assert_eq!(topic2, "mirror.text");
        assert_eq!(type2, "mirror_doc_deleted");
    }

    #[test]
    fn load_test_10k_events() {
        let conn = crate::db::open(Utf8Path::new(":memory:")).unwrap();
        let bus = EventBus::new(
            &BusBounds {
                source_fs: 10,
                mirror_text: 10,
            },
            Arc::new(Mutex::new(conn)),
        );
        let rx = bus.subscribe_source();
        let producer = bus.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..10_000 {
                producer.publish_source(SourceEvent::SyncStarted).unwrap();
            }
        });
        for _ in 0..10_000 {
            rx.recv().unwrap();
        }
        handle.join().unwrap();
    }
}

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::lock::Mutex;

pub type KeyValues = Arc<Mutex<BTreeMap<String, String>>>;

pub type App = app_http::App<crate::TypeConfig, crate::StateMachineStore, KeyValues>;

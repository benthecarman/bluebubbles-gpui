//! The unread count on the dock icon, through the Unity LauncherEntry
//! signal that Ubuntu's dock and KDE's task manager read.

use crate::config::APP_ID;
use std::collections::HashMap;
use zbus::names::BusName;
use zbus::zvariant::Value;

const PATH: &str = "/com/canonical/unity/launcherentry/bluebubbles_gpui";

pub async fn set(connection: &zbus::Connection, count: usize) -> zbus::Result<()> {
    let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
    properties.insert("count", Value::I64(count as i64));
    properties.insert("count-visible", Value::Bool(count > 0));
    connection
        .emit_signal(
            None::<BusName<'_>>,
            PATH,
            "com.canonical.Unity.LauncherEntry",
            "Update",
            &(format!("application://{APP_ID}.desktop"), properties),
        )
        .await
}

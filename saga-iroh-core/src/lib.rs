//! JNI surface for Saga Iroh call sessions on Android.
//! Callbacks into Kotlin via org.saga.iroh.IrohNativeBridge.

use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jint, JNI_VERSION_1_6};
use jni::{JNIEnv, JavaVM};
use log::{error, info, warn};

#[cfg(target_os = "android")]
use android_logger::Config;

#[cfg(feature = "iroh-transport")]
mod iroh_transport;

fn init_logging() {
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("SagaIrohCore"),
        );
    }
}

fn jstring_to_rust(env: &mut JNIEnv, input: &JString) -> Option<String> {
    let java_str = env.get_string(input).ok()?;
    Some(java_str.into())
}

static BRIDGE_CLASS: OnceLock<GlobalRef> = OnceLock::new();

const BRIDGE_CLASS_NAME: &str = "org/saga/iroh/IrohNativeBridge";

fn clear_pending_exception(env: &mut JNIEnv, context: &str) {
    if env.exception_check().unwrap_or(false) {
        if let Err(e) = env.exception_describe() {
            error!("{context}: exception_describe failed: {e}");
        }
        let _ = env.exception_clear();
    }
}

fn bridge_jclass<'local>(env: &mut JNIEnv<'local>) -> Option<JClass<'local>> {
    let class_ref = BRIDGE_CLASS.get()?;
    let local = env.new_local_ref(class_ref.as_obj()).ok()?;
    Some(JClass::from(local))
}

fn notify_connected(env: &mut JNIEnv, session_id: &str) {
    let Some(jclass) = bridge_jclass(env) else {
        error!("IrohNativeBridge class not cached");
        return;
    };
    let Ok(jid) = env.new_string(session_id) else {
        return;
    };
    if let Err(e) = env.call_static_method(
        &jclass,
        "notifyConnected",
        "(Ljava/lang/String;)V",
        &[JValue::from(&jid)],
    ) {
        error!("notifyConnected failed: {e}");
    }
    clear_pending_exception(env, "notifyConnected");
}

fn notify_failed(env: &mut JNIEnv, session_id: &str, reason: &str) {
    let Some(jclass) = bridge_jclass(env) else {
        error!("IrohNativeBridge class not cached");
        return;
    };
    let Ok(jid) = env.new_string(session_id) else {
        return;
    };
    let Ok(jreason) = env.new_string(reason) else {
        return;
    };
    if let Err(e) = env.call_static_method(
        &jclass,
        "notifyFailed",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[JValue::from(&jid), JValue::from(&jreason)],
    ) {
        error!("notifyFailed failed: {e}");
    }
    clear_pending_exception(env, "notifyFailed");
}

fn ensure_bridge_class_cached(env: &mut JNIEnv) {
    if BRIDGE_CLASS.get().is_some() {
        return;
    }
    match env.find_class(BRIDGE_CLASS_NAME) {
        Ok(class) => match env.new_global_ref(&class) {
            Ok(global) => {
                let _ = BRIDGE_CLASS.set(global);
                info!("cached IrohNativeBridge jclass via find_class");
            }
            Err(e) => error!("failed to cache IrohNativeBridge global ref: {e}"),
        },
        Err(e) => error!("find_class {BRIDGE_CLASS_NAME} failed: {e}"),
    }
}

fn connect_peer(peer_id: &str, session_id: &str) -> Result<(), String> {
    info!("connect_peer peer=[{peer_id}] session=[{session_id}]");

    #[cfg(feature = "iroh-transport")]
    {
        return iroh_transport::connect(peer_id, session_id);
    }

    #[cfg(not(feature = "iroh-transport"))]
    {
        if peer_id.to_ascii_lowercase().contains("fail") {
            return Err("native connect failure (peer id contains 'fail')".into());
        }
        thread::sleep(Duration::from_millis(300));
        info!("native stub transport connected peer=[{peer_id}]");
        Ok(())
    }
}

#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _: *mut std::ffi::c_void) -> jint {
    init_logging();
    info!(
        "saga-iroh-core JNI_OnLoad (iroh-transport={})",
        cfg!(feature = "iroh-transport")
    );
    match vm.attach_current_thread() {
        Ok(_env) => {}
        Err(e) => error!("JNI_OnLoad attach_current_thread failed: {e}"),
    }
    JNI_VERSION_1_6
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeConnect(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
    peer_id: JString,
) {
    ensure_bridge_class_cached(&mut env);
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return;
    };
    let Some(peer) = jstring_to_rust(&mut env, &peer_id) else {
        return;
    };

    let vm = match env.get_java_vm() {
        Ok(vm) => vm,
        Err(e) => {
            error!("get_java_vm failed: {e}");
            return;
        }
    };

    thread::spawn(move || {
        let result = connect_peer(&peer, &session);
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                error!("attach_current_thread failed: {e}");
                return;
            }
        };
        match result {
            Ok(()) => notify_connected(&mut env, &session),
            Err(reason) => {
                warn!("connect failed session=[{session}] reason=[{reason}]");
                notify_failed(&mut env, &session, &reason);
            }
        }
    });
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeDisconnect(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return;
    };
    info!("nativeDisconnect session=[{session}]");
    #[cfg(feature = "iroh-transport")]
    iroh_transport::disconnect(&session);
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetForceHandshakeFail(
    _env: JNIEnv,
    _thiz: JObject,
    force: jni::sys::jboolean,
) {
    #[cfg(feature = "iroh-transport")]
    iroh_transport::set_force_handshake_fail(force == jni::sys::JNI_TRUE);
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativePollHandshake(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) -> jint {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return 2;
    };
    #[cfg(feature = "iroh-transport")]
    {
        return iroh_transport::poll_handshake(&session) as jint;
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = session;
        1
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMediaRoundTripOk(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) -> jni::sys::jboolean {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return jni::sys::JNI_FALSE;
    };
    #[cfg(feature = "iroh-transport")]
    {
        return if iroh_transport::media_round_trip_ok(&session) {
            jni::sys::JNI_TRUE
        } else {
            jni::sys::JNI_FALSE
        };
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = session;
        jni::sys::JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeIsAvailable(
    mut env: JNIEnv,
    _thiz: JObject,
) -> jni::sys::jboolean {
    ensure_bridge_class_cached(&mut env);
    jni::sys::JNI_TRUE
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetRelayUrl(
    mut env: JNIEnv,
    _thiz: JObject,
    relay_url: JString,
) {
    let Some(url) = jstring_to_rust(&mut env, &relay_url) else {
        return;
    };
    #[cfg(feature = "iroh-transport")]
    iroh_transport::set_relay_url(&url);
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = url;
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativePollRelayReady(
    _env: JNIEnv,
    _thiz: JObject,
) -> jint {
    #[cfg(feature = "iroh-transport")]
    {
        return match iroh_transport::poll_relay_ready() {
            iroh_transport::RelayPoll::Pending => 0,
            iroh_transport::RelayPoll::Ready => 1,
            iroh_transport::RelayPoll::Failed => 2,
        };
    }
    #[cfg(not(feature = "iroh-transport"))]
    1
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetDevIdentity(
    mut env: JNIEnv,
    _thiz: JObject,
    peer_label: JString,
) {
    ensure_bridge_class_cached(&mut env);
    let Some(label) = jstring_to_rust(&mut env, &peer_label) else {
        return;
    };
    #[cfg(feature = "iroh-transport")]
    {
        iroh_transport::set_dev_identity(&label);
        if let Err(e) = iroh_transport::ensure_listening() {
            error!("ensure_listening after setDevIdentity failed: {e}");
        }
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = label;
        warn!("nativeSetDevIdentity ignored (iroh-transport feature disabled)");
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeLocalEndpointId(
    env: JNIEnv,
    _thiz: JObject,
) -> jni::sys::jstring {
    #[cfg(feature = "iroh-transport")]
    {
        match iroh_transport::local_endpoint_id_hex() {
            Ok(id) => env.new_string(id).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut()),
            Err(e) => {
                error!("nativeLocalEndpointId failed: {e}");
                std::ptr::null_mut()
            }
        }
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = env;
        std::ptr::null_mut()
    }
}

//! JNI surface for Saga Iroh call sessions on Android.
//! Callbacks into Kotlin via org.saga.iroh.IrohNativeBridge.

use std::sync::OnceLock;
use std::thread;

#[cfg(not(feature = "iroh-transport"))]
use std::time::Duration;

use jni::objects::{GlobalRef, JByteBuffer, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong, JNI_VERSION_1_6};
use jni::{JNIEnv, JavaVM};
use log::{error, info, warn};

#[cfg(target_os = "android")]
use android_logger::Config;

#[cfg(feature = "iroh-transport")]
mod iroh_transport;

mod media_bridge;

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
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

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

#[allow(dead_code)]
fn notify_incoming_call(session_id: &str, lookup_key: &str, remote_endpoint_id: &str) {
    let Some(vm) = JAVA_VM.get() else {
        error!("notifyIncomingCall: JavaVM not cached");
        return;
    };
    let sid = session_id.to_string();
    let lookup = lookup_key.to_string();
    let remote = remote_endpoint_id.to_string();
    thread::spawn(move || {
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                error!("notifyIncomingCall attach failed: {e}");
                return;
            }
        };
        let Some(jclass) = bridge_jclass(&mut env) else {
            error!("notifyIncomingCall: IrohNativeBridge class not cached");
            return;
        };
        let Ok(jsid) = env.new_string(&sid) else {
            return;
        };
        let Ok(jlookup) = env.new_string(&lookup) else {
            return;
        };
        let Ok(jremote) = env.new_string(&remote) else {
            return;
        };
        if let Err(e) = env.call_static_method(
            &jclass,
            "notifyIncomingCall",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
            &[
                JValue::from(&jsid),
                JValue::from(&jlookup),
                JValue::from(&jremote),
            ],
        ) {
            error!("notifyIncomingCall failed: {e}");
        }
        clear_pending_exception(&mut env, "notifyIncomingCall");
    });
}

#[cfg(feature = "iroh-transport")]
pub(crate) fn bridge_notify_incoming_call(
    session_id: &str,
    lookup_key: &str,
    remote_endpoint_id: &str,
) {
    notify_incoming_call(session_id, lookup_key, remote_endpoint_id);
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
        iroh_transport::connect(peer_id, session_id)
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
    let _ = JAVA_VM.set(vm);
    match JAVA_VM.get().unwrap().attach_current_thread() {
        Ok(mut env) => ensure_bridge_class_cached(&mut env),
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
    let _ = force;
    #[cfg(feature = "iroh-transport")]
    iroh_transport::set_force_handshake_fail(force == jni::sys::JNI_TRUE);
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetForceBindingPerturb(
    _env: JNIEnv,
    _thiz: JObject,
    force: jni::sys::jboolean,
) {
    let _ = force;
    #[cfg(feature = "iroh-transport")]
    iroh_transport::set_force_binding_perturb(force == jni::sys::JNI_TRUE);
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
        iroh_transport::poll_handshake(&session) as jint
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
        if iroh_transport::media_round_trip_ok(&session) {
            jni::sys::JNI_TRUE
        } else {
            jni::sys::JNI_FALSE
        }
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
        match iroh_transport::poll_relay_ready() {
            iroh_transport::RelayPoll::Pending => 0,
            iroh_transport::RelayPoll::Ready => 1,
            iroh_transport::RelayPoll::Failed => 2,
        }
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
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeStartMedia(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
    mode: jint,
) -> jboolean {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return jni::sys::JNI_FALSE;
    };
    #[cfg(feature = "iroh-transport")]
    {
        let mode = match mode {
            1 => media_bridge::MediaMode::Srtp,
            2 => media_bridge::MediaMode::PlainRtp,
            _ => return jni::sys::JNI_FALSE,
        };
        let Some((role, hs, roq_conn)) = iroh_transport::take_media_setup(&session) else {
            log::warn!("nativeStartMedia: no RoQ setup for [{session}]");
            return jni::sys::JNI_FALSE;
        };
        let ok = iroh_transport::runtime().block_on(async move {
            let flow = saga_media::MediaPipeline::flow_id_hint();
            let roq = iroh_roq::Session::new(roq_conn);
            let send = match roq.new_send_flow(flow).await {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("RoQ send flow: {e}");
                    return false;
                }
            };
            let recv = match roq.new_receive_flow(flow).await {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("RoQ recv flow: {e}");
                    return false;
                }
            };
            media_bridge::start_media(&session, mode, role, hs.as_deref(), send, recv, roq)
        });
        if ok {
            jni::sys::JNI_TRUE
        } else {
            jni::sys::JNI_FALSE
        }
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = (session, mode);
        jni::sys::JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeStopMedia(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return;
    };
    media_bridge::stop_media(&session);
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativePushCapturePcm(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
    frame: JObject,
) {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return;
    };
    let buf = JByteBuffer::from(frame);
    let Ok(ptr) = env.get_direct_buffer_address(&buf) else {
        return;
    };
    let Ok(cap) = env.get_direct_buffer_capacity(&buf) else {
        return;
    };
    if cap == 0 {
        return;
    }
    // SAFETY: direct ByteBuffer from Kotlin audio thread; capacity checked.
    let slice = unsafe { std::slice::from_raw_parts(ptr, cap) };
    media_bridge::push_capture(&session, slice);
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativePullPlaybackPcm(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
    frame: JObject,
) -> jint {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return 0;
    };
    let buf = JByteBuffer::from(frame);
    let Ok(ptr) = env.get_direct_buffer_address(&buf) else {
        return 0;
    };
    let Ok(cap) = env.get_direct_buffer_capacity(&buf) else {
        return 0;
    };
    if cap == 0 {
        return 0;
    }
    let slice = unsafe { std::slice::from_raw_parts_mut(ptr, cap) };
    media_bridge::pull_playback(&session, slice) as jint
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetMediaMuted(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
    muted: jboolean,
) {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return;
    };
    media_bridge::set_muted(&session, muted == jni::sys::JNI_TRUE);
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetMediaHeld(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
    held: jboolean,
) {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return;
    };
    media_bridge::set_held(&session, held == jni::sys::JNI_TRUE);
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeRekeyMedia(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) -> jlong {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return -1;
    };
    #[cfg(feature = "iroh-transport")]
    {
        let Some(hs) = iroh_transport::handshake_output(&session) else {
            return -1;
        };
        media_bridge::rekey_media(&session, hs.as_ref())
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = session;
        -1
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
            Ok(id) => env
                .new_string(id)
                .map(|s| s.into_raw())
                .unwrap_or(std::ptr::null_mut()),
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

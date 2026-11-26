//! JNI surface for Saga Iroh call sessions on Android.
//! Callbacks into Kotlin via org.saga.iroh.IrohNativeBridge.

use std::sync::OnceLock;
use std::thread;

#[cfg(feature = "nitrokey")]
#[used]
static NITROKEY_BACKEND_MARKER: &str = saga_token_usb::NITROKEY_BACKEND_MARKER;

#[cfg(not(feature = "iroh-transport"))]
use std::time::Duration;

use jni::objects::{GlobalRef, JByteBuffer, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong, JNI_VERSION_1_6};
use jni::{JNIEnv, JavaVM};
use log::{error, info, warn};

#[cfg(target_os = "android")]
use android_logger::Config;

#[cfg(feature = "iroh-transport")]
mod android_dns;
#[cfg(feature = "iroh-transport")]
mod iroh_transport;
/// Native mesh (needs the token handshake, so `mock-token` until a production backend lands).
#[cfg(feature = "mock-token")]
mod mesh;
/// Encrypted 1:1 text messaging (issue #41 / Part 3).
#[cfg(feature = "iroh-transport")]
mod messaging;

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
static DNS_BRIDGE_CLASS: OnceLock<GlobalRef> = OnceLock::new();
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

const BRIDGE_CLASS_NAME: &str = "org/saga/iroh/IrohNativeBridge";
const DNS_BRIDGE_CLASS_NAME: &str = "org/saga/iroh/AndroidDnsBridge";

#[cfg(all(feature = "iroh-transport", target_os = "android"))]
pub(crate) fn java_vm() -> Option<&'static JavaVM> {
    JAVA_VM.get()
}

/// Cached `AndroidDnsBridge` jclass — FindClass fails on spawn_blocking threads.
#[cfg(all(feature = "iroh-transport", target_os = "android"))]
pub(crate) fn dns_bridge_jclass<'local>(env: &mut JNIEnv<'local>) -> Option<JClass<'local>> {
    let class_ref = DNS_BRIDGE_CLASS.get()?;
    let local = env.new_local_ref(class_ref.as_obj()).ok()?;
    Some(JClass::from(local))
}

fn clear_pending_exception(env: &mut JNIEnv, context: &str) {
    if env.exception_check().unwrap_or(false) {
        if let Err(e) = env.exception_describe() {
            error!("{context}: exception_describe failed: {e}");
        }
        let _ = env.exception_clear();
    }
}

#[cfg(all(feature = "iroh-transport", target_os = "android"))]
pub(crate) fn clear_pending_exception_pub(env: &mut JNIEnv, context: &str) {
    clear_pending_exception(env, context);
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

/// Native -> Kotlin: one mesh link changed state (`IrohNativeBridge.notifyMeshPeerState`).
/// Runs on a fresh attached thread, like `notify_incoming_call`.
#[cfg(feature = "mock-token")]
fn notify_mesh_peer_state(group_id: &str, link_id: &str, state: i32) {
    let Some(vm) = JAVA_VM.get() else {
        warn!("notifyMeshPeerState: JavaVM not cached (group=[{group_id}] link=[{link_id}] state={state})");
        return;
    };
    let group = group_id.to_string();
    let link = link_id.to_string();
    thread::spawn(move || {
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                error!("notifyMeshPeerState attach failed: {e}");
                return;
            }
        };
        let Some(jclass) = bridge_jclass(&mut env) else {
            error!("notifyMeshPeerState: IrohNativeBridge class not cached");
            return;
        };
        let Ok(jgroup) = env.new_string(&group) else {
            return;
        };
        let Ok(jlink) = env.new_string(&link) else {
            return;
        };
        if let Err(e) = env.call_static_method(
            &jclass,
            "notifyMeshPeerState",
            "(Ljava/lang/String;Ljava/lang/String;I)V",
            &[
                JValue::from(&jgroup),
                JValue::from(&jlink),
                JValue::from(state),
            ],
        ) {
            error!("notifyMeshPeerState failed: {e}");
        }
        clear_pending_exception(&mut env, "notifyMeshPeerState");
    });
}

#[cfg(feature = "mock-token")]
pub(crate) fn bridge_notify_mesh_peer_state(group_id: &str, link_id: &str, state: i32) {
    notify_mesh_peer_state(group_id, link_id, state);
}

#[cfg(feature = "iroh-transport")]
pub(crate) fn bridge_notify_incoming_call(
    session_id: &str,
    lookup_key: &str,
    remote_endpoint_id: &str,
) {
    notify_incoming_call(session_id, lookup_key, remote_endpoint_id);
}

/// Peer closed the pairwise QUIC session mid-call (G8). Not used for connect-phase failures.
#[cfg(feature = "iroh-transport")]
pub(crate) fn bridge_notify_session_closed(session_id: &str, reason: &str) {
    let Some(vm) = JAVA_VM.get() else {
        warn!("notifySessionClosed: JavaVM not cached (session=[{session_id}])");
        return;
    };
    let sid = session_id.to_string();
    let why = reason.to_string();
    thread::spawn(move || {
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                error!("notifySessionClosed attach failed: {e}");
                return;
            }
        };
        let Some(jclass) = bridge_jclass(&mut env) else {
            error!("notifySessionClosed: IrohNativeBridge class not cached");
            return;
        };
        let Ok(jid) = env.new_string(&sid) else {
            return;
        };
        let Ok(jreason) = env.new_string(&why) else {
            return;
        };
        if let Err(e) = env.call_static_method(
            &jclass,
            "notifySessionClosed",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::from(&jid), JValue::from(&jreason)],
        ) {
            error!("notifySessionClosed failed: {e}");
        }
        clear_pending_exception(&mut env, "notifySessionClosed");
    });
}

#[cfg(not(feature = "iroh-transport"))]
pub(crate) fn bridge_notify_session_closed(_session_id: &str, _reason: &str) {}

/// Pairwise signaling-only roster introduction (G4a).
/// Only referenced from the mock-token roster accept loop (debug/E2E builds).
#[cfg(all(feature = "iroh-transport", feature = "mock-token"))]
pub(crate) fn bridge_notify_roster_introduction(
    group_id: &str,
    lookup_key: &str,
    endpoint_id: &str,
    fingerprint_hex: &str,
    introducer_name: &str,
) {
    let Some(vm) = JAVA_VM.get() else {
        return;
    };
    let group_id = group_id.to_string();
    let lookup_key = lookup_key.to_string();
    let endpoint_id = endpoint_id.to_string();
    let fingerprint_hex = fingerprint_hex.to_string();
    let introducer_name = introducer_name.to_string();
    thread::spawn(move || {
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                error!("notifyRosterIntroduction attach failed: {e}");
                return;
            }
        };
        let Some(jclass) = bridge_jclass(&mut env) else {
            return;
        };
        let Ok(jgroup) = env.new_string(&group_id) else {
            return;
        };
        let Ok(jlookup) = env.new_string(&lookup_key) else {
            return;
        };
        let Ok(jep) = env.new_string(&endpoint_id) else {
            return;
        };
        let Ok(jfp) = env.new_string(&fingerprint_hex) else {
            return;
        };
        let Ok(jintro) = env.new_string(&introducer_name) else {
            return;
        };
        if let Err(e) = env.call_static_method(
            &jclass,
            "notifyRosterIntroduction",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
            &[
                JValue::from(&jgroup),
                JValue::from(&jlookup),
                JValue::from(&jep),
                JValue::from(&jfp),
                JValue::from(&jintro),
            ],
        ) {
            error!("notifyRosterIntroduction failed: {e}");
        }
        clear_pending_exception(&mut env, "notifyRosterIntroduction");
    });
}

/// §16: peer voice-disguise chip (informational only — never touches padlock state).
#[cfg(feature = "iroh-transport")]
pub(crate) fn notify_peer_disguise(session_id: &str, active: bool) {
    let Some(vm) = JAVA_VM.get() else {
        return;
    };
    let sid = session_id.to_string();
    thread::spawn(move || {
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                error!("notifyPeerDisguise attach failed: {e}");
                return;
            }
        };
        let Some(jclass) = bridge_jclass(&mut env) else {
            return;
        };
        let Ok(jsid) = env.new_string(&sid) else {
            return;
        };
        let jactive = if active {
            jni::sys::JNI_TRUE
        } else {
            jni::sys::JNI_FALSE
        };
        if let Err(e) = env.call_static_method(
            &jclass,
            "notifyPeerDisguise",
            "(Ljava/lang/String;Z)V",
            &[JValue::from(&jsid), JValue::Bool(jactive)],
        ) {
            error!("notifyPeerDisguise failed: {e}");
        }
        clear_pending_exception(&mut env, "notifyPeerDisguise");
    });
}

#[cfg(not(feature = "iroh-transport"))]
pub(crate) fn notify_peer_disguise(_session_id: &str, _active: bool) {}

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
    if BRIDGE_CLASS.get().is_none() {
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
    if DNS_BRIDGE_CLASS.get().is_none() {
        match env.find_class(DNS_BRIDGE_CLASS_NAME) {
            Ok(class) => match env.new_global_ref(&class) {
                Ok(global) => {
                    let _ = DNS_BRIDGE_CLASS.set(global);
                    info!("cached AndroidDnsBridge jclass via find_class");
                }
                Err(e) => error!("failed to cache AndroidDnsBridge global ref: {e}"),
            },
            Err(e) => error!("find_class {DNS_BRIDGE_CLASS_NAME} failed: {e}"),
        }
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

#[cfg(feature = "iroh-transport")]
fn notify_msg_delivered(peer_label: &str, text: &str) {
    let Some(vm) = JAVA_VM.get() else {
        error!("notifyMsgDelivered: JavaVM not cached");
        return;
    };
    let peer = peer_label.to_string();
    let body = text.to_string();
    thread::spawn(move || {
        let mut env = match vm.attach_current_thread() {
            Ok(env) => env,
            Err(e) => {
                error!("notifyMsgDelivered attach failed: {e}");
                return;
            }
        };
        let Some(jclass) = bridge_jclass(&mut env) else {
            error!("notifyMsgDelivered: IrohNativeBridge class not cached");
            return;
        };
        let Ok(jpeer) = env.new_string(&peer) else {
            return;
        };
        let Ok(jtext) = env.new_string(&body) else {
            return;
        };
        if let Err(e) = env.call_static_method(
            &jclass,
            "notifyMsgDelivered",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::from(&jpeer), JValue::from(&jtext)],
        ) {
            error!("notifyMsgDelivered failed: {e}");
        }
        clear_pending_exception(&mut env, "notifyMsgDelivered");
    });
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
    #[cfg(feature = "iroh-transport")]
    {
        messaging::set_on_delivered(|peer, text| {
            notify_msg_delivered(&peer, &text);
        });
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

/// Publish Application context to ndk_context so iroh can read system DNS when desired.
/// Keep a GlobalRef for process lifetime (required by install_android_jni_context).
#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeInstallAndroidContext(
    mut env: JNIEnv,
    _thiz: JObject,
    context: JObject,
) {
    ensure_bridge_class_cached(&mut env);
    #[cfg(all(target_os = "android", feature = "iroh-transport"))]
    {
        use std::ffi::c_void;
        use std::sync::OnceLock;
        static ANDROID_APP_CTX: OnceLock<GlobalRef> = OnceLock::new();
        if ANDROID_APP_CTX.get().is_some() {
            return;
        }
        let Ok(vm) = env.get_java_vm() else {
            error!("nativeInstallAndroidContext: get_java_vm failed");
            return;
        };
        let Ok(global) = env.new_global_ref(&context) else {
            error!("nativeInstallAndroidContext: new_global_ref failed");
            return;
        };
        let ctx_ptr = global.as_raw() as *mut c_void;
        let vm_ptr = vm.get_java_vm_pointer() as *mut c_void;
        if ANDROID_APP_CTX.set(global).is_err() {
            return;
        }
        // Safety: GlobalRef kept in ANDROID_APP_CTX until process exit; VM is process-scoped.
        unsafe {
            iroh::dns::install_android_jni_context(vm_ptr, ctx_ptr);
        }
        info!("android JNI context installed for iroh DNS");
    }
    #[cfg(not(all(target_os = "android", feature = "iroh-transport")))]
    {
        let _ = context;
    }
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
        #[cfg(feature = "mock-token")]
        if let Some(node) = mesh::global_node() {
            if let Some(ok) = node.start_media(&session, mode as i32) {
                return if ok {
                    jni::sys::JNI_TRUE
                } else {
                    jni::sys::JNI_FALSE
                };
            }
        }
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
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetMediaMutedAll(
    _env: JNIEnv,
    _thiz: JObject,
    muted: jboolean,
) {
    media_bridge::set_muted_all(muted == jni::sys::JNI_TRUE);
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
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetEchoMode(
    _env: JNIEnv,
    _thiz: JObject,
    enabled: jboolean,
) {
    #[cfg(feature = "iroh-transport")]
    media_bridge::set_echo_mode(enabled == jni::sys::JNI_TRUE);
    #[cfg(not(feature = "iroh-transport"))]
    let _ = enabled;
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetRxFault(
    _env: JNIEnv,
    _thiz: JObject,
    drop_pct: jint,
    jitter_ms: jint,
) {
    #[cfg(feature = "iroh-transport")]
    media_bridge::set_rx_fault(drop_pct.max(0) as u32, jitter_ms.max(0) as u32);
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = (drop_pct, jitter_ms);
    }
}

/// §16: push voice-disguise settings into the media engine (and advertise to peer).
#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSetDisguiseSettings(
    _env: JNIEnv,
    _thiz: JObject,
    mode: jint,
    pitch_semitones: jni::sys::jfloat,
    formant_strength: jint,
    random_each_call: jboolean,
    call_enabled: jboolean,
) {
    #[cfg(feature = "iroh-transport")]
    media_bridge::apply_disguise_settings(
        mode.clamp(0, 3) as u8,
        pitch_semitones,
        formant_strength.clamp(0, 2) as u8,
        random_each_call == jni::sys::JNI_TRUE,
        call_enabled == jni::sys::JNI_TRUE,
    );
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = (
            mode,
            pitch_semitones,
            formant_strength,
            random_each_call,
            call_enabled,
        );
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeLocalDisguiseActive(
    _env: JNIEnv,
    _thiz: JObject,
) -> jboolean {
    #[cfg(feature = "iroh-transport")]
    {
        if media_bridge::local_disguise_active() {
            jni::sys::JNI_TRUE
        } else {
            jni::sys::JNI_FALSE
        }
    }
    #[cfg(not(feature = "iroh-transport"))]
    jni::sys::JNI_FALSE
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativePeerDisguiseActive(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) -> jboolean {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return jni::sys::JNI_FALSE;
    };
    #[cfg(feature = "iroh-transport")]
    {
        if media_bridge::peer_disguise_active(&session) {
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

/// Offline / "Test my voice": apply current disguise settings to a direct PCM buffer.
#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeProcessDisguisePcm(
    env: JNIEnv,
    _thiz: JObject,
    pcm: JObject,
) {
    let buf = JByteBuffer::from(pcm);
    let Ok(ptr) = env.get_direct_buffer_address(&buf) else {
        return;
    };
    let Ok(cap) = env.get_direct_buffer_capacity(&buf) else {
        return;
    };
    if cap < 2 {
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts_mut(ptr, cap) };
    let n = cap / 2;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        samples.push(i16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]));
    }
    #[cfg(feature = "iroh-transport")]
    saga_media::disguise_process_buffer(&mut samples);
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = &samples;
    }
    for (i, s) in samples.iter().enumerate() {
        let b = s.to_le_bytes();
        bytes[i * 2] = b[0];
        bytes[i * 2 + 1] = b[1];
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeOnLocalAnswered(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) -> jboolean {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return jni::sys::JNI_FALSE;
    };
    if media_bridge::on_local_answered(&session) {
        jni::sys::JNI_TRUE
    } else {
        jni::sys::JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeIsSendArmed(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) -> jboolean {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return jni::sys::JNI_FALSE;
    };
    if media_bridge::is_send_armed(&session) {
        jni::sys::JNI_TRUE
    } else {
        jni::sys::JNI_FALSE
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

/// Local ephemeral-session fingerprint (64 lowercase hex) from the mock-token
/// identity label. Null when mock-token is not compiled in (release Nitrokey builds).
#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeLocalFingerprintHex(
    env: JNIEnv,
    _thiz: JObject,
) -> jni::sys::jstring {
    #[cfg(all(feature = "mock-token", feature = "iroh-transport"))]
    {
        let hex = mock_token::MockIdentity::from_label(&iroh_transport::local_identity_label())
            .fingerprint_hex_string();
        env.new_string(hex)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
    #[cfg(not(all(feature = "mock-token", feature = "iroh-transport")))]
    {
        let _ = env;
        std::ptr::null_mut()
    }
}

// ---------------------------------------------------------------------------
// Native mesh JNI (see mesh.rs). Without `mock-token` there is no token
// backend yet, so the mesh reports unavailable and Kotlin keeps the pairwise
// 1:1 scaffold.
// ---------------------------------------------------------------------------

fn jbool(v: bool) -> jboolean {
    if v {
        jni::sys::JNI_TRUE
    } else {
        jni::sys::JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMeshIsAvailable(
    mut env: JNIEnv,
    _thiz: JObject,
) -> jboolean {
    ensure_bridge_class_cached(&mut env);
    jbool(cfg!(feature = "mock-token"))
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMeshOpen(
    mut env: JNIEnv,
    _thiz: JObject,
    group_id: JString,
) -> jboolean {
    ensure_bridge_class_cached(&mut env);
    let Some(group) = jstring_to_rust(&mut env, &group_id) else {
        return jni::sys::JNI_FALSE;
    };
    #[cfg(feature = "mock-token")]
    {
        match mesh::ensure_global_node() {
            Ok(node) => jbool(node.open(&group)),
            Err(e) => {
                error!("nativeMeshOpen group=[{group}] failed: {e}");
                jni::sys::JNI_FALSE
            }
        }
    }
    #[cfg(not(feature = "mock-token"))]
    {
        let _ = group;
        jni::sys::JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMeshAddPeer(
    mut env: JNIEnv,
    _thiz: JObject,
    group_id: JString,
    link_id: JString,
    peer_id: JString,
) -> jboolean {
    ensure_bridge_class_cached(&mut env);
    let (Some(group), Some(link), Some(peer)) = (
        jstring_to_rust(&mut env, &group_id),
        jstring_to_rust(&mut env, &link_id),
        jstring_to_rust(&mut env, &peer_id),
    ) else {
        return jni::sys::JNI_FALSE;
    };
    #[cfg(feature = "mock-token")]
    {
        match mesh::ensure_global_node() {
            Ok(node) => jbool(node.add_peer(&group, &link, &peer)),
            Err(e) => {
                error!("nativeMeshAddPeer group=[{group}] link=[{link}] failed: {e}");
                jni::sys::JNI_FALSE
            }
        }
    }
    #[cfg(not(feature = "mock-token"))]
    {
        let _ = (group, link, peer);
        jni::sys::JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMeshRemovePeer(
    mut env: JNIEnv,
    _thiz: JObject,
    group_id: JString,
    link_id: JString,
) {
    let (Some(group), Some(link)) = (
        jstring_to_rust(&mut env, &group_id),
        jstring_to_rust(&mut env, &link_id),
    ) else {
        return;
    };
    #[cfg(feature = "mock-token")]
    if let Some(node) = mesh::global_node() {
        node.remove_peer(&group, &link);
    }
    #[cfg(not(feature = "mock-token"))]
    let _ = (group, link);
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMeshClose(
    mut env: JNIEnv,
    _thiz: JObject,
    group_id: JString,
) {
    let Some(group) = jstring_to_rust(&mut env, &group_id) else {
        return;
    };
    #[cfg(feature = "mock-token")]
    if let Some(node) = mesh::global_node() {
        node.close(&group);
    }
    #[cfg(not(feature = "mock-token"))]
    let _ = group;
}

/// Authenticated long-term fingerprint for a pairwise (1:1) session, or null.
#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeSessionPeerFingerprint(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
) -> jni::sys::jstring {
    let Some(session) = jstring_to_rust(&mut env, &session_id) else {
        return std::ptr::null_mut();
    };
    #[cfg(feature = "iroh-transport")]
    {
        match iroh_transport::peer_fingerprint(&session) {
            Some(hex) => env
                .new_string(hex)
                .map(|s| s.into_raw())
                .unwrap_or(std::ptr::null_mut()),
            None => std::ptr::null_mut(),
        }
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = (session, env);
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeAnnounceIntroducedPeer(
    mut env: JNIEnv,
    _thiz: JObject,
    session_id: JString,
    group_id: JString,
    lookup_key: JString,
    endpoint_id: JString,
    fingerprint_hex: JString,
    introducer_name: JString,
) -> jboolean {
    let (Some(session), Some(group), Some(lookup), Some(endpoint), Some(fp), Some(intro)) = (
        jstring_to_rust(&mut env, &session_id),
        jstring_to_rust(&mut env, &group_id),
        jstring_to_rust(&mut env, &lookup_key),
        jstring_to_rust(&mut env, &endpoint_id),
        jstring_to_rust(&mut env, &fingerprint_hex),
        jstring_to_rust(&mut env, &introducer_name),
    ) else {
        return jni::sys::JNI_FALSE;
    };
    #[cfg(feature = "iroh-transport")]
    {
        jbool(iroh_transport::announce_introduced_peer(
            &session, &group, &lookup, &endpoint, &fp, &intro,
        ))
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = (session, group, lookup, endpoint, fp, intro);
        jni::sys::JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMeshPeerFingerprint(
    mut env: JNIEnv,
    _thiz: JObject,
    group_id: JString,
    link_id: JString,
) -> jni::sys::jstring {
    let (Some(group), Some(link)) = (
        jstring_to_rust(&mut env, &group_id),
        jstring_to_rust(&mut env, &link_id),
    ) else {
        return std::ptr::null_mut();
    };
    #[cfg(feature = "mock-token")]
    {
        let fp = mesh::global_node().and_then(|n| n.peer_fingerprint(&group, &link));
        match fp {
            Some(hex) => env
                .new_string(hex)
                .map(|s| s.into_raw())
                .unwrap_or(std::ptr::null_mut()),
            None => std::ptr::null_mut(),
        }
    }
    #[cfg(not(feature = "mock-token"))]
    {
        let _ = (group, link, env);
        std::ptr::null_mut()
    }
}

/// Send encrypted text to a peer label / EndpointId. Returns a status string
/// (`delivered` / `sent` / `TOKEN_NEEDED` / `HANDSHAKE_FAILED` / `NOT_REACHABLE:…`).
#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMsgSend(
    mut env: JNIEnv,
    _thiz: JObject,
    peer_id: JString,
    text: JString,
) -> jni::sys::jstring {
    let (Some(peer), Some(body)) = (
        jstring_to_rust(&mut env, &peer_id),
        jstring_to_rust(&mut env, &text),
    ) else {
        return env
            .new_string("MALFORMED")
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut());
    };
    #[cfg(feature = "iroh-transport")]
    {
        let status = messaging::send_text(&peer, &body);
        env.new_string(status)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = (peer, body);
        env.new_string("TOKEN_NEEDED")
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }
}

#[no_mangle]
pub extern "system" fn Java_org_saga_iroh_IrohNativeBridge_nativeMsgSetForceTokenUnavailable(
    _env: JNIEnv,
    _thiz: JObject,
    force: jboolean,
) {
    #[cfg(feature = "iroh-transport")]
    {
        messaging::set_force_token_unavailable(force == jni::sys::JNI_TRUE);
    }
    #[cfg(not(feature = "iroh-transport"))]
    {
        let _ = force;
    }
}

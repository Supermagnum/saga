//! Android-backed iroh [`DnsResolver`] that uses `android.net.DnsResolver` via JNI.
//!
//! Honours Private DNS (DoT), VPN, and Orbot. No third-party resolvers are pinned.
//!
//! # Discovery choice
//!
//! Keep `presets::N0` (PkarrPublisher over HTTPS + DnsAddressLookup TXT) and route
//! **all** of iroh's DNS (A/AAAA for relay hostnames and TXT for pkarr discovery)
//! through this backend. Pkarr-over-HTTPS resolve alone would still need A/AAAA for
//! `*.relay.n0.iroh.link` / `dns.iroh.link`, so a platform DnsResolver is required
//! either way; implementing TXT here keeps one path that respects user DNS settings.
//!
//! Android concurrency: the Kotlin bridge allows a small number of in-flight
//! DnsResolver calls (not fully serial) so relay + discovery lookups can overlap;
//! unbounded bursts have been seen to ECONNREFUSED on AVDs with Private DNS.

use std::fmt;
use std::future::Future;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use iroh::dns::{BoxIter, DnsError, DnsResolver, Resolver, TxtRecordData};
use n0_error::e;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Pluggable backend so host unit tests can mock without JNI.
pub trait DnsBackend: fmt::Debug + Send + Sync + 'static {
    fn lookup_ipv4(&self, host: &str, timeout: Duration) -> Result<Vec<Ipv4Addr>, String>;
    fn lookup_ipv6(&self, host: &str, timeout: Duration) -> Result<Vec<Ipv6Addr>, String>;
    fn lookup_txt(&self, host: &str, timeout: Duration) -> Result<Vec<String>, String>;
}

/// Default per-lookup budget (matches Android bridge default).
/// Used by the Android backend and host unit tests; host lib builds only.
#[allow(dead_code)]
pub const DEFAULT_LOOKUP_TIMEOUT: Duration = Duration::from_secs(8);

/// iroh [`Resolver`] wrapping a [`DnsBackend`].
#[derive(Debug, Clone)]
pub struct PlatformDnsResolver<B: DnsBackend> {
    backend: Arc<B>,
    timeout: Duration,
}

impl<B: DnsBackend> PlatformDnsResolver<B> {
    /// Construct with [`DEFAULT_LOOKUP_TIMEOUT`]. Host lib builds only exercise
    /// this via unit tests / the Android target.
    #[allow(dead_code)]
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
            timeout: DEFAULT_LOOKUP_TIMEOUT,
        }
    }

    #[allow(dead_code)]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[allow(dead_code)]
    pub fn into_iroh(self) -> DnsResolver {
        DnsResolver::custom(self)
    }
}

fn map_join<E: std::fmt::Display>(e: E) -> DnsError {
    e!(
        DnsError::Resolve,
        std::io::Error::other(e.to_string()).into()
    )
}

fn map_backend(e: String) -> DnsError {
    e!(DnsError::Resolve, std::io::Error::other(e).into())
}

impl<B: DnsBackend> Resolver for PlatformDnsResolver<B> {
    fn lookup_ipv4(&self, host: String) -> BoxFuture<Result<BoxIter<Ipv4Addr>, DnsError>> {
        let backend = Arc::clone(&self.backend);
        let timeout = self.timeout;
        Box::pin(async move {
            let addrs = tokio::task::spawn_blocking(move || backend.lookup_ipv4(&host, timeout))
                .await
                .map_err(map_join)?
                .map_err(map_backend)?;
            if addrs.is_empty() {
                return Err(e!(DnsError::NoResponse));
            }
            let iter: BoxIter<Ipv4Addr> = Box::new(addrs.into_iter());
            Ok(iter)
        })
    }

    fn lookup_ipv6(&self, host: String) -> BoxFuture<Result<BoxIter<Ipv6Addr>, DnsError>> {
        let backend = Arc::clone(&self.backend);
        let timeout = self.timeout;
        Box::pin(async move {
            let addrs = tokio::task::spawn_blocking(move || backend.lookup_ipv6(&host, timeout))
                .await
                .map_err(map_join)?
                .map_err(map_backend)?;
            // Empty AAAA is fine — dual-stack callers fall back to A.
            let iter: BoxIter<Ipv6Addr> = Box::new(addrs.into_iter());
            Ok(iter)
        })
    }

    fn lookup_txt(&self, host: String) -> BoxFuture<Result<BoxIter<TxtRecordData>, DnsError>> {
        let backend = Arc::clone(&self.backend);
        let timeout = self.timeout;
        Box::pin(async move {
            let texts = tokio::task::spawn_blocking(move || backend.lookup_txt(&host, timeout))
                .await
                .map_err(map_join)?
                .map_err(map_backend)?;
            if texts.is_empty() {
                return Err(e!(DnsError::NoResponse));
            }
            let records: Vec<TxtRecordData> = texts
                .into_iter()
                .map(|s| TxtRecordData::from(vec![s.into_bytes().into_boxed_slice()]))
                .collect();
            let iter: BoxIter<TxtRecordData> = Box::new(records.into_iter());
            Ok(iter)
        })
    }

    fn clear_cache(&self) {}

    fn reset(&self) -> Box<dyn Resolver> {
        Box::new(Self {
            backend: Arc::clone(&self.backend),
            timeout: self.timeout,
        })
    }
}

/// JNI-backed backend (Android only).
#[cfg(target_os = "android")]
#[derive(Debug, Default, Clone, Copy)]
pub struct AndroidDnsBackend;

#[cfg(target_os = "android")]
impl DnsBackend for AndroidDnsBackend {
    fn lookup_ipv4(&self, host: &str, timeout: Duration) -> Result<Vec<Ipv4Addr>, String> {
        let arrays = call_byte_arrays("lookupIpv4", host, timeout)?;
        arrays
            .into_iter()
            .map(|b| {
                if b.len() != 4 {
                    return Err(format!("bad IPv4 len {}", b.len()));
                }
                Ok(Ipv4Addr::new(b[0], b[1], b[2], b[3]))
            })
            .collect()
    }

    fn lookup_ipv6(&self, host: &str, timeout: Duration) -> Result<Vec<Ipv6Addr>, String> {
        let arrays = call_byte_arrays("lookupIpv6", host, timeout)?;
        arrays
            .into_iter()
            .map(|b| {
                let arr: [u8; 16] = b.as_slice().try_into().map_err(|_| "bad IPv6 len")?;
                Ok(Ipv6Addr::from(arr))
            })
            .collect()
    }

    fn lookup_txt(&self, host: &str, timeout: Duration) -> Result<Vec<String>, String> {
        call_string_array("lookupTxt", host, timeout)
    }
}

#[cfg(target_os = "android")]
fn call_byte_arrays(method: &str, host: &str, timeout: Duration) -> Result<Vec<Vec<u8>>, String> {
    use jni::objects::{JByteArray, JObjectArray, JValue};

    let vm = crate::java_vm().ok_or_else(|| "JAVA_VM not set".to_string())?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("attach: {e}"))?;
    // Must use JNI_OnLoad-cached GlobalRef — FindClass fails off the app classloader.
    let class = crate::dns_bridge_jclass(&mut env)
        .ok_or_else(|| "AndroidDnsBridge jclass not cached".to_string())?;
    let jhost = env
        .new_string(host)
        .map_err(|e| format!("new_string: {e}"))?;
    let timeout_ms = i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX);
    let result = env
        .call_static_method(
            &class,
            method,
            "(Ljava/lang/String;J)[[B",
            &[JValue::Object(&jhost), JValue::Long(timeout_ms)],
        )
        .map_err(|e| {
            crate::clear_pending_exception_pub(&mut env, method);
            format!("{method}: {e}")
        })?;
    let obj = result.l().map_err(|e| format!("jobject: {e}"))?;
    if obj.is_null() {
        return Ok(Vec::new());
    }
    let arr: JObjectArray<'_> = unsafe { JObjectArray::from_raw(obj.into_raw()) };
    let len = env
        .get_array_length(&arr)
        .map_err(|e| format!("array len: {e}"))?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let elem = env
            .get_object_array_element(&arr, i)
            .map_err(|e| format!("elem: {e}"))?;
        let jba: JByteArray<'_> = unsafe { JByteArray::from_raw(elem.into_raw()) };
        let bytes = env
            .convert_byte_array(&jba)
            .map_err(|e| format!("bytes: {e}"))?;
        out.push(bytes);
    }
    Ok(out)
}

#[cfg(target_os = "android")]
fn call_string_array(method: &str, host: &str, timeout: Duration) -> Result<Vec<String>, String> {
    use jni::objects::{JObjectArray, JString, JValue};

    let vm = crate::java_vm().ok_or_else(|| "JAVA_VM not set".to_string())?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("attach: {e}"))?;
    let class = crate::dns_bridge_jclass(&mut env)
        .ok_or_else(|| "AndroidDnsBridge jclass not cached".to_string())?;
    let jhost = env
        .new_string(host)
        .map_err(|e| format!("new_string: {e}"))?;
    let timeout_ms = i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX);
    let result = env
        .call_static_method(
            &class,
            method,
            "(Ljava/lang/String;J)[Ljava/lang/String;",
            &[JValue::Object(&jhost), JValue::Long(timeout_ms)],
        )
        .map_err(|e| {
            crate::clear_pending_exception_pub(&mut env, method);
            format!("{method}: {e}")
        })?;
    let obj = result.l().map_err(|e| format!("jobject: {e}"))?;
    if obj.is_null() {
        return Ok(Vec::new());
    }
    let arr: JObjectArray<'_> = unsafe { JObjectArray::from_raw(obj.into_raw()) };
    let len = env
        .get_array_length(&arr)
        .map_err(|e| format!("array len: {e}"))?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let elem = env
            .get_object_array_element(&arr, i)
            .map_err(|e| format!("elem: {e}"))?;
        let jstr = JString::from(elem);
        let s = env
            .get_string(&jstr)
            .map_err(|e| format!("get_string: {e}"))?;
        out.push(String::from(s));
    }
    Ok(out)
}

/// Build the DnsResolver used when binding the Android endpoint.
#[cfg(target_os = "android")]
pub fn android_iroh_dns_resolver() -> DnsResolver {
    log::info!("using Android DnsResolver backend (Private DNS / VPN / Orbot)");
    PlatformDnsResolver::new(AndroidDnsBackend).into_iroh()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MockBackend {
        ipv4: Mutex<HashMap<String, Vec<Ipv4Addr>>>,
        ipv6: Mutex<HashMap<String, Vec<Ipv6Addr>>>,
        txt: Mutex<HashMap<String, Vec<String>>>,
        fail: Mutex<HashMap<String, String>>,
    }

    impl DnsBackend for MockBackend {
        fn lookup_ipv4(&self, host: &str, _timeout: Duration) -> Result<Vec<Ipv4Addr>, String> {
            if let Some(err) = self.fail.lock().unwrap().get(host) {
                return Err(err.clone());
            }
            Ok(self
                .ipv4
                .lock()
                .unwrap()
                .get(host)
                .cloned()
                .unwrap_or_default())
        }

        fn lookup_ipv6(&self, host: &str, _timeout: Duration) -> Result<Vec<Ipv6Addr>, String> {
            if let Some(err) = self.fail.lock().unwrap().get(host) {
                return Err(err.clone());
            }
            Ok(self
                .ipv6
                .lock()
                .unwrap()
                .get(host)
                .cloned()
                .unwrap_or_default())
        }

        fn lookup_txt(&self, host: &str, _timeout: Duration) -> Result<Vec<String>, String> {
            if let Some(err) = self.fail.lock().unwrap().get(host) {
                return Err(err.clone());
            }
            Ok(self
                .txt
                .lock()
                .unwrap()
                .get(host)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn mock_backend_resolves_a_and_txt() {
        let mock = MockBackend::default();
        mock.ipv4.lock().unwrap().insert(
            "euc1-1.relay.n0.iroh.link".into(),
            vec![Ipv4Addr::new(91, 99, 237, 97)],
        );
        mock.txt.lock().unwrap().insert(
            "_iroh.example".into(),
            vec!["relay=https://euc1-1.relay.n0.iroh.link./".into()],
        );
        let resolver = PlatformDnsResolver::new(mock).into_iroh();

        let mut addrs = resolver
            .lookup_ipv4("euc1-1.relay.n0.iroh.link", Duration::from_secs(1))
            .await
            .expect("A lookup");
        assert_eq!(addrs.next().unwrap(), Ipv4Addr::new(91, 99, 237, 97));

        let mut txts = resolver
            .lookup_txt("_iroh.example", Duration::from_secs(1))
            .await
            .expect("TXT lookup");
        let first = txts.next().unwrap().to_string();
        assert!(first.contains("relay=https://"));
    }

    #[tokio::test]
    async fn mock_backend_empty_is_no_response() {
        let resolver = PlatformDnsResolver::new(MockBackend::default()).into_iroh();
        let res = resolver
            .lookup_ipv4("missing.example", Duration::from_secs(1))
            .await;
        assert!(matches!(res, Err(DnsError::NoResponse { .. })));
    }

    #[tokio::test]
    async fn mock_backend_error_maps_to_resolve() {
        let mock = MockBackend::default();
        mock.fail
            .lock()
            .unwrap()
            .insert("bad.example".into(), "boom".into());
        let resolver = PlatformDnsResolver::new(mock).into_iroh();
        let res = resolver
            .lookup_ipv4("bad.example", Duration::from_secs(1))
            .await;
        assert!(matches!(res, Err(DnsError::Resolve { .. })));
    }
}

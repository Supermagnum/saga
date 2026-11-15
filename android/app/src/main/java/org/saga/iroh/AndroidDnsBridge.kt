package org.saga.iroh

import android.content.Context
import android.net.ConnectivityManager
import android.net.DnsResolver
import android.net.Network
import android.net.NetworkCapabilities
import android.os.CancellationSignal
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * Platform DNS via [DnsResolver] (API 29+).
 *
 * Honours Private DNS (DoT via netd), VPN, and Orbot. No in-app third-party
 * resolvers.
 *
 * Concurrency: at most [MAX_INFLIGHT] lookups in flight. Unbounded parallel
 * DnsResolver bursts on some AVDs return ECONNREFUSED after the first success;
 * a small permit count keeps relay + discovery overlapping without that flake.
 *
 * On default-network change: remember the active [Network], flush the
 * address/TXT cache, and pass that Network explicitly to DnsResolver
 * query/rawQuery. Does **not** call [ConnectivityManager.bindProcessToNetwork]
 * (process-wide bind broke local-relay UDP on API 29 AVDs with
 * PermissionDenied on sendmsg).
 *
 * Discovery choice: keep `presets::N0` (PkarrPublisher HTTPS + DnsAddressLookup
 * TXT) and route all A/AAAA/TXT through this backend.
 */
object AndroidDnsBridge {
    private const val TAG = "[Saga Android DNS]"
    private const val DEFAULT_TIMEOUT_MS = 8_000L
    private const val QTYPE_TXT = 16
    private const val QCLASS_IN = 1
    private const val CACHE_TTL_MS = 120_000L
    /** Cap parallel DnsResolver calls (AVD DoT flakes under large bursts). */
    private const val MAX_INFLIGHT = 3

    @Volatile
    private var appContext: Context? = null

    @Volatile
    private var cachedNetwork: Network? = null

    private val inflight = Semaphore(MAX_INFLIGHT)
    private val addrCache = ConcurrentHashMap<String, CacheEntry<List<InetAddress>>>()
    private val txtCache = ConcurrentHashMap<String, CacheEntry<List<String>>>()

    private val handlerThread: HandlerThread = HandlerThread("saga-dns").also { it.start() }
    private val dnsHandler = Handler(handlerThread.looper)
    private val dnsExecutor: Executor = Executor { cmd -> dnsHandler.post(cmd) }

    private data class CacheEntry<T>(val value: T, val atMs: Long) {
        fun fresh(now: Long = System.currentTimeMillis()): Boolean = now - atMs < CACHE_TTL_MS
    }

    @JvmStatic
    fun install(context: Context) {
        val app = context.applicationContext
        appContext = app
        val cm = app.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        applyNetwork(cm.activeNetwork ?: firstValidated(cm), reason = "install")
        try {
            cm.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) {
                    applyNetwork(network, reason = "onAvailable")
                }

                override fun onCapabilitiesChanged(
                    network: Network,
                    networkCapabilities: NetworkCapabilities
                ) {
                    if (networkCapabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)) {
                        applyNetwork(network, reason = "onCapabilitiesChanged/validated")
                    }
                }

                override fun onLost(network: Network) {
                    if (cachedNetwork == network) {
                        val next = cm.activeNetwork ?: firstValidated(cm)
                        applyNetwork(next, reason = "onLost")
                    }
                }
            })
        } catch (e: RuntimeException) {
            Log.w(TAG, "registerDefaultNetworkCallback failed: ${e.message}")
        }
        Log.i(TAG, "AndroidDnsBridge installed network=$cachedNetwork maxInflight=$MAX_INFLIGHT")
    }

    /**
     * Track [network] for DnsResolver only (no process-wide bind). Flush the
     * DNS cache when the network identity changes so stale A/AAAA/TXT from the
     * previous path are not reused.
     */
    private fun applyNetwork(network: Network?, reason: String) {
        val prev = cachedNetwork
        cachedNetwork = network
        if (prev != network) {
            flushCache()
            Log.i(TAG, "network change ($reason): $prev -> $network (DNS cache flushed)")
        }
    }

    @JvmStatic
    fun flushCache() {
        addrCache.clear()
        txtCache.clear()
    }

    /**
     * Network passed explicitly to [DnsResolver] (API 29+). Never binds the
     * process; may be null → DnsResolver uses the system default network.
     */
    private fun resolveNetwork(): Network? {
        val ctx = appContext ?: return cachedNetwork
        val cm = ctx.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val active = cm.activeNetwork
        if (active != null && active != cachedNetwork) {
            applyNetwork(active, reason = "resolveNetwork/refresh")
        }
        return cachedNetwork ?: active ?: firstValidated(cm)
    }

    /**
     * Best-effort A-record warm for public N0 hosts. Call off the main thread.
     * Stops early after [budgetMs] so local-relay boot is not blocked forever.
     */
    @JvmStatic
    fun prefetchN0HostsBlocking(budgetMs: Long = 20_000L) {
        val deadline = System.currentTimeMillis() + budgetMs
        val hosts = listOf(
            "dns.iroh.link",
            "euc1-1.relay.n0.iroh.link",
            "use1-1.relay.n0.iroh.link",
            "usw1-1.relay.n0.iroh.link",
            "aps1-1.relay.n0.iroh.link"
        )
        for (h in hosts) {
            if (System.currentTimeMillis() >= deadline) break
            try {
                lookupIpv4(h, 3_000L)
            } catch (t: Throwable) {
                Log.w(TAG, "prefetch $h: ${t.message}")
            }
        }
    }

    private fun normalizeHost(host: String): String = host.trim().trimEnd('.')

    @JvmStatic
    fun lookupIpv4(host: String, timeoutMs: Long = DEFAULT_TIMEOUT_MS): Array<ByteArray> {
        return lookupAddresses(normalizeHost(host), DnsResolver.TYPE_A, timeoutMs)
            .filterIsInstance<Inet4Address>()
            .map { it.address }
            .toTypedArray()
    }

    @JvmStatic
    fun lookupIpv6(host: String, timeoutMs: Long = DEFAULT_TIMEOUT_MS): Array<ByteArray> {
        return lookupAddresses(normalizeHost(host), DnsResolver.TYPE_AAAA, timeoutMs)
            .filterIsInstance<Inet6Address>()
            .map { it.address }
            .toTypedArray()
    }

    @JvmStatic
    fun lookupTxt(host: String, timeoutMs: Long = DEFAULT_TIMEOUT_MS): Array<String> {
        val name = normalizeHost(host)
        txtCache[name]?.takeIf { it.fresh() }?.let { return it.value.toTypedArray() }
        withPermit(timeoutMs) {
            txtCache[name]?.takeIf { it.fresh() }?.let { return it.value.toTypedArray() }
            val network = resolveNetwork()
            val query = buildDnsQuery(name, QTYPE_TXT)
            val answer = rawQuery(network, query, timeoutMs)
            val texts = parseTxtAnswers(answer, name)
            txtCache[name] = CacheEntry(texts, System.currentTimeMillis())
            return texts.toTypedArray()
        }
    }

    private fun lookupAddresses(host: String, nsType: Int, timeoutMs: Long): List<InetAddress> {
        val key = "$host|$nsType"
        addrCache[key]?.takeIf { it.fresh() }?.let { return it.value }
        withPermit(timeoutMs) {
            addrCache[key]?.takeIf { it.fresh() }?.let { return it.value }
            val network = resolveNetwork()
            var last: Throwable? = null
            val attempts = if (timeoutMs <= 3_500L) 2 else 5
            repeat(attempts) { attempt ->
                try {
                    val addrs = dnsResolverQuery(network, host, nsType, timeoutMs)
                    if (addrs.isNotEmpty()) {
                        addrCache[key] = CacheEntry(addrs, System.currentTimeMillis())
                        Log.d(TAG, "ok host=$host type=$nsType n=${addrs.size} net=$network")
                        return addrs
                    }
                } catch (t: Throwable) {
                    last = t
                    Log.w(TAG, "attempt=${attempt + 1} host=$host type=$nsType: ${t.message}")
                    try {
                        Thread.sleep(200L * (attempt + 1))
                    } catch (_: InterruptedException) {
                    }
                }
            }
            if (nsType == DnsResolver.TYPE_AAAA) {
                addrCache[key] = CacheEntry(emptyList(), System.currentTimeMillis())
                return emptyList()
            }
            throw DnsBridgeException("no addresses for $host type=$nsType (${last?.message})")
        }
    }

    private inline fun <T> withPermit(timeoutMs: Long, block: () -> T): T {
        val waitMs = timeoutMs.coerceAtLeast(1_000L)
        if (!inflight.tryAcquire(waitMs, TimeUnit.MILLISECONDS)) {
            throw DnsBridgeException("DNS permit timed out (${MAX_INFLIGHT} in flight)")
        }
        try {
            return block()
        } finally {
            inflight.release()
        }
    }

    private fun dnsResolverQuery(
        network: Network?,
        host: String,
        nsType: Int,
        timeoutMs: Long
    ): List<InetAddress> {
        val latch = CountDownLatch(1)
        val result = AtomicReference<Result<List<InetAddress>>>(
            Result.failure(DnsBridgeException("pending"))
        )
        val cancel = CancellationSignal()
        DnsResolver.getInstance().query(
            network,
            host,
            nsType,
            DnsResolver.FLAG_EMPTY,
            dnsExecutor,
            cancel,
            object : DnsResolver.Callback<List<InetAddress>> {
                override fun onAnswer(answer: List<InetAddress>, rcode: Int) {
                    result.set(Result.success(answer))
                    latch.countDown()
                }

                override fun onError(error: DnsResolver.DnsException) {
                    result.set(Result.failure(error))
                    latch.countDown()
                }
            }
        )
        if (!latch.await(timeoutMs, TimeUnit.MILLISECONDS)) {
            cancel.cancel()
            throw DnsBridgeException("DnsResolver timed out for $host type=$nsType")
        }
        return result.get().getOrElse { throw it }
    }

    private fun rawQuery(network: Network?, query: ByteArray, timeoutMs: Long): ByteArray {
        val latch = CountDownLatch(1)
        val result = AtomicReference<Result<ByteArray>>(Result.failure(DnsBridgeException("pending")))
        val cancel = CancellationSignal()
        DnsResolver.getInstance().rawQuery(
            network,
            query,
            DnsResolver.FLAG_EMPTY,
            dnsExecutor,
            cancel,
            object : DnsResolver.Callback<ByteArray> {
                override fun onAnswer(answer: ByteArray, rcode: Int) {
                    result.set(Result.success(answer))
                    latch.countDown()
                }

                override fun onError(error: DnsResolver.DnsException) {
                    result.set(Result.failure(error))
                    latch.countDown()
                }
            }
        )
        if (!latch.await(timeoutMs, TimeUnit.MILLISECONDS)) {
            cancel.cancel()
            throw DnsBridgeException("rawQuery timed out")
        }
        return result.get().getOrElse { throw it }
    }

    private fun firstValidated(cm: ConnectivityManager): Network? {
        @Suppress("DEPRECATION")
        for (network in cm.allNetworks) {
            val caps = cm.getNetworkCapabilities(network) ?: continue
            if (caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
                caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
            ) {
                return network
            }
        }
        return null
    }

    private fun buildDnsQuery(host: String, qtype: Int): ByteArray {
        val labels = host.trimEnd('.').split('.').filter { it.isNotEmpty() }
        val nameLen = labels.sumOf { 1 + it.toByteArray(Charsets.US_ASCII).size } + 1
        val buf = ByteBuffer.allocate(12 + nameLen + 4).order(ByteOrder.BIG_ENDIAN)
        buf.putShort(0x1234.toShort())
        buf.putShort(0x0100.toShort())
        buf.putShort(1)
        buf.putShort(0)
        buf.putShort(0)
        buf.putShort(0)
        for (label in labels) {
            val bytes = label.toByteArray(Charsets.US_ASCII)
            require(bytes.size <= 63) { "label too long" }
            buf.put(bytes.size.toByte())
            buf.put(bytes)
        }
        buf.put(0)
        buf.putShort(qtype.toShort())
        buf.putShort(QCLASS_IN.toShort())
        return buf.array()
    }

    private fun parseTxtAnswers(packet: ByteArray, expectedHost: String): List<String> {
        if (packet.size < 12) throw DnsBridgeException("DNS answer too short")
        val buf = ByteBuffer.wrap(packet).order(ByteOrder.BIG_ENDIAN)
        buf.position(4)
        val qd = buf.short.toInt() and 0xffff
        val an = buf.short.toInt() and 0xffff
        buf.short
        buf.short
        repeat(qd) { skipName(buf); buf.short; buf.short }
        val out = ArrayList<String>()
        repeat(an) {
            skipName(buf)
            val type = buf.short.toInt() and 0xffff
            buf.short
            buf.int
            val rdlen = buf.short.toInt() and 0xffff
            if (type == QTYPE_TXT) {
                val end = buf.position() + rdlen
                while (buf.position() < end) {
                    val len = buf.get().toInt() and 0xff
                    val bytes = ByteArray(len)
                    buf.get(bytes)
                    out.add(String(bytes, Charsets.UTF_8))
                }
            } else {
                buf.position(buf.position() + rdlen)
            }
        }
        if (out.isEmpty()) throw DnsBridgeException("no TXT answers for $expectedHost")
        return out
    }

    private fun skipName(buf: ByteBuffer) {
        while (true) {
            val len = buf.get().toInt() and 0xff
            when {
                len == 0 -> return
                (len and 0xc0) == 0xc0 -> {
                    buf.get()
                    return
                }
                else -> buf.position(buf.position() + len)
            }
        }
    }

    class DnsBridgeException(message: String) : Exception(message)
}

package org.saga

import android.app.Application
import android.util.Log
import org.saga.audio.SagaPcmDebugReceiver
import org.saga.contacts.TestContactSeeder
import org.saga.iroh.DevIdentityStore
import org.saga.iroh.IrohIncomingCallBridge
import org.saga.iroh.IrohNativeBridge
import org.saga.iroh.IrohRelayStatus
import org.saga.iroh.IrohRelayStore
import org.saga.telecom.SagaPhoneAccountRegistrar

class SagaApplication : Application() {
    companion object {
        private const val TAG = "[Saga Application]"
    }

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "SagaApplication started (contact-keys=phone-labels-v3)")
        SagaPhoneAccountRegistrar.register(this)
        SagaPhoneAccountRegistrar.ensureEnabled(this)
        IrohIncomingCallBridge.init(this)
        IrohRelayStatus.registerQueryReceiver(this)
        SagaPcmDebugReceiver.register(this)
        // Load native first (JNI_OnLoad caches AndroidDnsBridge), then install
        // the Kotlin bridge + ndk_context before any endpoint bind.
        // DNS goes through android.net.DnsResolver (Private DNS / VPN / Orbot).
        // No pinned third-party resolvers.
        IrohNativeBridge.installAndroidContext(this)
        org.saga.iroh.AndroidDnsBridge.install(this)
        val customRelay = IrohRelayStore.getUrl(this)
        customRelay?.let { relayUrl ->
            Log.i(TAG, "Using custom Iroh relay url=[$relayUrl]")
            IrohNativeBridge.setRelayUrl(relayUrl)
        }
        IrohNativeBridge.setForceHandshakeFail(
            org.saga.handshake.SagaTestFlags.isForceHandshakeFail(this)
        )
        IrohNativeBridge.setForceBindingPerturb(
            org.saga.handshake.SagaTestFlags.isForceBindingPerturb(this)
        )
        TestContactSeeder.seedIfMissing(this)
        val identityLabel = DevIdentityStore.getLabel(this)
        if (identityLabel == null) {
            Log.w(TAG, "No dev identity label — Iroh listener will not bind until identity is set")
            return
        }
        // Public N0: warm DNS off the main looper, then bind. Custom/local relay:
        // bind immediately (no public hostname lookups required for online()).
        if (customRelay.isNullOrBlank()) {
            Thread({
                org.saga.iroh.AndroidDnsBridge.prefetchN0HostsBlocking(20_000L)
                Log.i(TAG, "Provisioning Iroh dev identity after DNS prefetch")
                IrohNativeBridge.setDevIdentity(identityLabel)
            }, "saga-iroh-boot").apply { isDaemon = true }.start()
        } else {
            Log.i(TAG, "Provisioning Iroh dev identity on app start")
            IrohNativeBridge.setDevIdentity(identityLabel)
        }
    }
}

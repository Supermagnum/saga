package org.saga

import android.app.Application
import android.util.Log
import org.saga.contacts.TestContactSeeder
import org.saga.iroh.DevIdentityStore
import org.saga.iroh.IrohNativeBridge
import org.saga.telecom.SagaPhoneAccountRegistrar

class SagaApplication : Application() {
    companion object {
        private const val TAG = "[Saga Application]"
    }

    override fun onCreate() {
        super.onCreate()
        Log.i(TAG, "SagaApplication started (contact-keys=phone-labels-v3)")
        SagaPhoneAccountRegistrar.register(this)
        DevIdentityStore.getLabel(this)?.let { label ->
            Log.i(TAG, "Provisioning Iroh dev identity on app start")
            IrohNativeBridge.setDevIdentity(label)
        } ?: Log.w(TAG, "No dev identity label — Iroh listener will not bind until identity is set")
        IrohNativeBridge.setForceHandshakeFail(
            org.saga.handshake.SagaTestFlags.isForceHandshakeFail(this)
        )
        TestContactSeeder.seedIfMissing(this)
    }
}

package org.saga.token

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Build
import android.util.Log
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * Finds a supported USB CCID token, requests permission, and exchanges APDUs.
 * Native code calls [transmitApdu] via JNI when the nitrokey feature is linked.
 */
object TokenUsbBridge {
    private const val TAG = "TokenUsbBridge"
    private const val ACTION_USB_PERMISSION = "org.saga.token.USB_PERMISSION"

    @Volatile
    private var transport: UsbCcidTransport? = null

    fun findSupportedDevice(context: Context): UsbDevice? {
        val mgr = context.getSystemService(Context.USB_SERVICE) as UsbManager
        return mgr.deviceList.values.firstOrNull { UsbCcidTransport.isSupported(it) }
    }

    fun ensurePermission(context: Context, device: UsbDevice, timeoutMs: Long = 30_000): Boolean {
        val mgr = context.getSystemService(Context.USB_SERVICE) as UsbManager
        if (mgr.hasPermission(device)) return true
        val latch = CountDownLatch(1)
        val granted = AtomicReference(false)
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context?, intent: Intent?) {
                if (intent?.action != ACTION_USB_PERMISSION) return
                granted.set(intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false))
                latch.countDown()
            }
        }
        val filter = IntentFilter(ACTION_USB_PERMISSION)
        if (Build.VERSION.SDK_INT >= 33) {
            context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("UnspecifiedRegisterReceiverFlag")
            context.registerReceiver(receiver, filter)
        }
        val piFlags = PendingIntent.FLAG_UPDATE_CURRENT or
            if (Build.VERSION.SDK_INT >= 23) PendingIntent.FLAG_IMMUTABLE else 0
        val pi = PendingIntent.getBroadcast(context, 0, Intent(ACTION_USB_PERMISSION), piFlags)
        mgr.requestPermission(device, pi)
        val ok = latch.await(timeoutMs, TimeUnit.MILLISECONDS) && granted.get()
        try {
            context.unregisterReceiver(receiver)
        } catch (_: Exception) {
        }
        return ok
    }

    fun open(context: Context): Boolean {
        close()
        val device = findSupportedDevice(context) ?: return false
        if (!ensurePermission(context, device)) return false
        val mgr = context.getSystemService(Context.USB_SERVICE) as UsbManager
        transport = UsbCcidTransport.open(mgr, device)
        return transport != null
    }

    fun close() {
        transport?.close()
        transport = null
    }

    /** JNI entry: CAPDU in, RAPDU out (including SW1SW2). */
    @JvmStatic
    fun transmitApdu(capdu: ByteArray): ByteArray {
        val t = transport ?: throw UsbTokenException("token not open")
        return t.transmit(capdu)
    }

    fun isOpen(): Boolean = transport != null

    fun logCardAbsent() {
        Log.i(TAG, "No supported USB token attached")
    }
}

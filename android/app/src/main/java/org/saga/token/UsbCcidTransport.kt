package org.saga.token

import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbEndpoint
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import android.util.Log
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicInteger

/**
 * Minimal CCID PC_to_RDR_XfrBlock / RDR_to_PC_DataBlock APDU transport for
 * OpenPGP cards (Nitrokey, future Galdralag Dabao). Shared USB path for Part 4.
 */
class UsbCcidTransport(
    private val connection: UsbDeviceConnection,
    private val usbInterface: UsbInterface,
    private val bulkOut: UsbEndpoint,
    private val bulkIn: UsbEndpoint,
) {
    private val seq = AtomicInteger(0)

    fun transmit(capdu: ByteArray): ByteArray {
        val bSeq = (seq.getAndIncrement() and 0xff).toByte()
        val header = ByteBuffer.allocate(10).order(ByteOrder.LITTLE_ENDIAN)
        header.put(0x6f.toByte()) // PC_to_RDR_XfrBlock
        header.putInt(capdu.size)
        header.put(0) // slot
        header.put(bSeq)
        header.put(0) // bBWI
        header.putShort(0) // wLevelParameter
        val out = header.array() + capdu
        val written = connection.bulkTransfer(bulkOut, out, out.size, TIMEOUT_MS)
        if (written != out.size) {
            throw UsbTokenException("CCID bulk OUT wrote $written / ${out.size}")
        }
        val buf = ByteArray(MAX_CCID)
        val read = connection.bulkTransfer(bulkIn, buf, buf.size, TIMEOUT_MS)
        if (read < 10) {
            throw UsbTokenException("CCID bulk IN short ($read)")
        }
        if (buf[0] != 0x80.toByte()) {
            throw UsbTokenException("unexpected CCID msg 0x${buf[0].toUByte().toString(16)}")
        }
        val dwLength = ByteBuffer.wrap(buf, 1, 4).order(ByteOrder.LITTLE_ENDIAN).int
        if (dwLength < 0 || 10 + dwLength > read) {
            throw UsbTokenException("CCID length $dwLength vs read $read")
        }
        return buf.copyOfRange(10, 10 + dwLength)
    }

    fun close() {
        try {
            connection.releaseInterface(usbInterface)
        } catch (_: Exception) {
        }
        connection.close()
    }

    companion object {
        private const val TAG = "UsbCcidTransport"
        private const val TIMEOUT_MS = 4000
        private const val MAX_CCID = 2048

        /** VID/PID allow-list (decimal matches usb_device_filter.xml). */
        fun isSupported(device: UsbDevice): Boolean {
            val vid = device.vendorId
            val pid = device.productId
            return (vid == 0x20a0 && (pid == 0x42b2 || pid == 0x4108)) ||
                (vid == 0x1d50 && pid == 0x6197)
        }

        fun open(manager: UsbManager, device: UsbDevice): UsbCcidTransport? {
            if (!isSupported(device)) return null
            val conn = manager.openDevice(device) ?: return null
            var claimed: UsbInterface? = null
            var bulkOut: UsbEndpoint? = null
            var bulkIn: UsbEndpoint? = null
            for (i in 0 until device.interfaceCount) {
                val intf = device.getInterface(i)
                var out: UsbEndpoint? = null
                var inn: UsbEndpoint? = null
                for (e in 0 until intf.endpointCount) {
                    val ep = intf.getEndpoint(e)
                    if (ep.type != UsbConstants.USB_ENDPOINT_XFER_BULK) continue
                    if (ep.direction == UsbConstants.USB_DIR_OUT) out = ep
                    if (ep.direction == UsbConstants.USB_DIR_IN) inn = ep
                }
                if (out != null && inn != null && conn.claimInterface(intf, true)) {
                    claimed = intf
                    bulkOut = out
                    bulkIn = inn
                    break
                }
            }
            if (claimed == null || bulkOut == null || bulkIn == null) {
                Log.w(TAG, "No CCID bulk interface on ${device.deviceName}")
                conn.close()
                return null
            }
            return UsbCcidTransport(conn, claimed, bulkOut, bulkIn)
        }
    }
}

class UsbTokenException(message: String) : Exception(message)

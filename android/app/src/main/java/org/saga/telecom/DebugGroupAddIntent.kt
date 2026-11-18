package org.saga.telecom

import android.content.Context
import android.content.Intent
import android.util.Log
import android.widget.Toast
import org.saga.dial.DialTarget
import org.saga.dial.DialTargetResolver
import org.saga.iroh.IrohDialManager

/**
 * Shared debug handler for [ACTION_ADD_PARTICIPANT] so both MainActivity and
 * SagaInCallActivity can run [GroupCallCoordinator.requestAdd] without burying
 * the live InCall UI (which previously emptied soleLiveLeg visibility).
 */
object DebugGroupAddIntent {
    const val ACTION_ADD_PARTICIPANT = "org.saga.TEST_ADD_PARTICIPANT"
    private const val TAG = "[Saga DebugGroupAdd]"

    fun maybeHandle(context: Context, intent: Intent?): Boolean {
        if (intent?.action != ACTION_ADD_PARTICIPANT) return false
        if ((context.applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) == 0) {
            Log.e(TAG, "TEST_ADD_PARTICIPANT refused — not a debuggable build")
            return true
        }
        val name = intent.getStringExtra("add_contact")
            ?: intent.getStringExtra("contact_name")
            ?: intent.getStringExtra("lookup_key")
            ?: return true
        val confirmUnencrypted =
            intent.getBooleanExtra("confirm_unencrypted", false) ||
                intent.getStringExtra("confirm_unencrypted")
                    ?.equals("true", ignoreCase = true) == true
        Log.i(
            TAG,
            "E2E add request add_contact=[${intent.getStringExtra("add_contact")}] " +
                "contact_name=[${intent.getStringExtra("contact_name")}] " +
                "lookup_key=[${intent.getStringExtra("lookup_key")}] " +
                "confirm_unencrypted=[$confirmUnencrypted] " +
                "nonce=[${intent.getStringExtra("e2e_nonce")}] resolved=[$name] " +
                "connections=[${SagaIrohConnectionRegistry.all().size}] " +
                "sessions=[${IrohDialManager.get(context).sessionCount()}]"
        )
        val target = DialTargetResolver.fromContactDisplayName(context, name)
            ?: DialTargetResolver.fromInput(context, name, preferCellular = false)
        if (target !is DialTarget.Iroh) {
            Log.e(TAG, "TEST_ADD_PARTICIPANT requires an Iroh contact, got [$target]")
            Toast.makeText(context, "Iroh contact required", Toast.LENGTH_SHORT).show()
            Log.i(TAG, "E2E add participant name=[$name] outcome=[REFUSED_NOT_IROH]")
            return true
        }
        val outcome = GroupCallCoordinator.requestAdd(
            context,
            GroupCallCoordinator.AddRequest(
                target = target,
                confirmedUnencrypted = confirmUnencrypted,
                introducerName = intent.getStringExtra("introduced_by"),
                rosterFingerprintHex = intent.getStringExtra("roster_fingerprint"),
                groupId = intent.getStringExtra("conference")
            )
        )
        // Keep the same log line the harness scrapes (Main Activity tag historically).
        Log.i(
            "[Saga Main Activity]",
            "E2E add participant name=[$name] outcome=[$outcome]"
        )
        if (outcome == GroupCallCoordinator.AddOutcome.REFUSED_MAX_PARTICIPANTS) {
            Toast.makeText(context, "Call is full", Toast.LENGTH_LONG).show()
        }
        return true
    }
}

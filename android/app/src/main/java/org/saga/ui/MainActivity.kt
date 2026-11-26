package org.saga.ui

import android.app.role.RoleManager
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.ContactsContract
import android.util.Log
import android.view.View
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import org.saga.R
import org.saga.databinding.ActivityMainBinding
import org.saga.dial.DialTarget
import org.saga.dial.DialTargetResolver
import org.saga.iroh.SagaDialer
import org.saga.iroh.IrohRelayStatus
import org.saga.security.midcall.MidCallRehandshakeOutcome
import org.saga.security.midcall.SagaMidCallRehandshakeController
import org.saga.telecom.SagaPhoneAccountRegistrar

class MainActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "[Saga Main Activity]"
        private const val PREFS = "saga_phone_account_ui"
        private const val KEY_ONBOARDING_SHOWN = "onboarding_shown"
        /** Open dialer for a second call from InCall; finish after placeCall. */
        const val EXTRA_ADD_CALL_MODE = "org.saga.EXTRA_ADD_CALL_MODE"
    }

    private lateinit var binding: ActivityMainBinding
    private var selectedContactUri: Uri? = null
    private var addCallMode = false

    private val requestDialerRole = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult()
    ) { result ->
        updateRoleStatus()
        refreshPhoneAccountUi(launchOnboardingIfNeeded = true)
        if (result.resultCode != RESULT_OK) {
            Toast.makeText(this, R.string.dialer_role_denied, Toast.LENGTH_SHORT).show()
        }
    }

    private val pickContact = registerForActivityResult(
        ActivityResultContracts.PickContact()
    ) { uri ->
        if (uri == null) return@registerForActivityResult
        selectedContactUri = uri
        val display = resolveContactDisplayName(uri)
        binding.peerIdInput.setText(display ?: uri.toString())
        Log.i(TAG, "Selected contact uri=[$uri] display=[$display]")
    }

    private val requestContactsPermission = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { granted ->
        if (granted) {
            pickContact.launch(null)
        } else {
            Toast.makeText(this, R.string.contacts_permission_denied, Toast.LENGTH_SHORT).show()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityMainBinding.inflate(layoutInflater)
        setContentView(binding.root)
        binding.requestDialerRole.setOnClickListener { requestDefaultDialerRole() }
        binding.pickContact.setOnClickListener { launchContactPicker() }
        binding.dialIroh.setOnClickListener { placeFromInput(preferCellular = false) }
        binding.dialUnencrypted.setOnClickListener { placeFromInput(preferCellular = true) }
        binding.openMessages.setOnClickListener {
            startActivity(Intent(this, org.saga.msg.ConversationListActivity::class.java))
        }
        binding.messageContact.setOnClickListener { openMessageForInput() }
        binding.openVoiceDisguise.setOnClickListener {
            startActivity(Intent(this, VoiceDisguiseSettingsActivity::class.java))
        }
        binding.openMyIdentity.setOnClickListener {
            startActivity(Intent(this, MyIdentityActivity::class.java))
        }
        binding.openSagaKeys.setOnClickListener {
            startActivity(Intent(this, SagaKeysActivity::class.java))
        }
        binding.openPrivacy.setOnClickListener {
            startActivity(
                Intent(
                    Intent.ACTION_VIEW,
                    Uri.parse(getString(R.string.privacy_readme_url)),
                ),
            )
        }
        binding.openLicences.setOnClickListener {
            startActivity(Intent(this, LicencesActivity::class.java))
        }
        binding.phoneAccountBanner.setOnClickListener {
            startActivity(Intent(this, PhoneAccountOnboardingActivity::class.java))
        }
        org.saga.audio.VoiceDisguiseStore.pushNative(this)
        wireKeypad()
        applyAddCallMode(intent)
        updateRoleStatus()
        handleIncomingIntent(intent, autoPlace = intent.action == Intent.ACTION_CALL)
    }

    private fun applyAddCallMode(intent: Intent?) {
        addCallMode = intent?.getBooleanExtra(EXTRA_ADD_CALL_MODE, false) == true
        if (addCallMode) {
            binding.title.text = getString(R.string.add_call_title)
            binding.addCallSubtitle.visibility = View.VISIBLE
            binding.addCallSubtitle.text = getString(R.string.keypad_hint)
            binding.requestDialerRole.visibility = View.GONE
            binding.roleStatus.visibility = View.GONE
            binding.dialIroh.text = getString(R.string.add_call_place)
            // Prefer cellular for add-call (second leg is typically carrier).
            binding.dialUnencrypted.visibility = View.VISIBLE
            Log.i(TAG, "Add-call dialer mode active")
        } else {
            binding.addCallSubtitle.visibility = View.GONE
        }
    }

    private fun wireKeypad() {
        val keys = listOf(
            binding.key1 to "1", binding.key2 to "2", binding.key3 to "3",
            binding.key4 to "4", binding.key5 to "5", binding.key6 to "6",
            binding.key7 to "7", binding.key8 to "8", binding.key9 to "9",
            binding.keyStar to "*", binding.key0 to "0", binding.keyHash to "#"
        )
        keys.forEach { (btn, digit) ->
            btn.setOnClickListener {
                selectedContactUri = null
                binding.peerIdInput.append(digit)
            }
        }
    }

    override fun onResume() {
        super.onResume()
        refreshPhoneAccountUi(launchOnboardingIfNeeded = true)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        // Deep-copy action+extras so SINGLE_TOP redelivery cannot share a mutable Bundle
        // with a prior TEST_CONTACT_CALL (seen as ADD action + stale contact_name=bob).
        val fresh = Intent(intent).setAction(intent.action)
        intent.extras?.let { fresh.replaceExtras(android.os.Bundle(it)) }
        setIntent(fresh)
        applyAddCallMode(fresh)
        if (fresh.action == SagaPhoneAccountRegistrar.ACTION_SHOW_ONBOARDING) {
            startActivity(Intent(this, PhoneAccountOnboardingActivity::class.java))
            return
        }
        handleIncomingIntent(fresh, autoPlace = fresh.action == Intent.ACTION_CALL)
    }

    private fun refreshPhoneAccountUi(launchOnboardingIfNeeded: Boolean) {
        SagaPhoneAccountRegistrar.register(this)
        val enabled = SagaPhoneAccountRegistrar.isAccountEnabled(this)
        binding.phoneAccountBanner.visibility = if (enabled) View.GONE else View.VISIBLE
        if (enabled) {
            getSharedPreferences(PREFS, MODE_PRIVATE)
                .edit()
                .putBoolean(KEY_ONBOARDING_SHOWN, true)
                .apply()
            return
        }
        Log.i(TAG, "PhoneAccount disabled — banner visible")
        if (!launchOnboardingIfNeeded) return
        val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
        val shown = prefs.getBoolean(KEY_ONBOARDING_SHOWN, false)
        if (!shown || intent?.action == SagaPhoneAccountRegistrar.ACTION_SHOW_ONBOARDING) {
            prefs.edit().putBoolean(KEY_ONBOARDING_SHOWN, true).apply()
            startActivity(Intent(this, PhoneAccountOnboardingActivity::class.java))
        }
    }

    private fun handleIncomingIntent(intent: Intent?, autoPlace: Boolean) {
        if (intent == null) return
        when (intent.action) {
            SagaPhoneAccountRegistrar.ACTION_SHOW_ONBOARDING -> {
                startActivity(Intent(this, PhoneAccountOnboardingActivity::class.java))
            }
            "org.saga.TEST_ANSWER_INCOMING" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_ANSWER_INCOMING refused — not a debuggable build")
                    return
                }
                val lookupKey = intent.getStringExtra("lookup_key") ?: return
                val mgr = org.saga.iroh.IrohDialManager.get(this)
                val callKey = mgr.findTelecomCallKey(lookupKey) ?: lookupKey
                Log.i(TAG, "E2E answer incoming lookup=[$lookupKey] callKey=[$callKey]")
                mgr.onIncomingAnswered(callKey)
            }
            "org.saga.TEST_CONTACT_CALL" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_CONTACT_CALL refused — not a debuggable build")
                    return
                }
                val name = intent.getStringExtra("contact_name") ?: return
                val preferCellular = intent.getBooleanExtra("force_unencrypted", false)
                selectedContactUri = null
                binding.peerIdInput.setText(name)
                Log.i(
                    TAG,
                    "E2E contact dial request name=[$name] force_unencrypted=[$preferCellular]"
                )
                // Bypass RoleManager dialer-role UI gate: OEM tablets (Samsung) often cannot
                // grant ROLE_DIALER via adb. placeCall still needs an enabled PhoneAccount.
                placeFromInputForDebug(preferCellular = preferCellular)
            }
            "org.saga.TEST_MSG_SEND" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_MSG_SEND refused — not a debuggable build")
                    return
                }
                val rawPeer = intent.getStringExtra("peer")
                    ?: intent.getStringExtra("contact_name")
                    ?: return
                val text = intent.getStringExtra("text") ?: return
                // Resolve display names to digit peer ids (Iroh mock-token labels).
                val target = DialTargetResolver.fromInput(this, rawPeer, preferCellular = false)
                val peer = when (target) {
                    is DialTarget.Iroh -> target.peerId.raw
                    is DialTarget.Cellular -> target.number.filter { it.isDigit() }
                    else -> rawPeer.filter { it.isDigit() }.ifBlank { rawPeer }
                }
                Log.i(
                    TAG,
                    "E2E msg send peer=[$rawPeer] iroh=[$peer] bytes=${text.length}"
                )
                try {
                    org.saga.msg.MessageService.enqueueOutbound(this, peer, text)
                } catch (e: IllegalStateException) {
                    Log.e(TAG, "E2E msg send refused: ${e.message}")
                }
            }
            "org.saga.TEST_ADD_PARTICIPANT" -> {
                // Prefer delivering this via SagaInCallActivity (keeps the live leg visible).
                // MainActivity still accepts it for older harnesses; restore InCall after.
                org.saga.telecom.DebugGroupAddIntent.maybeHandle(this, intent)
                startActivity(Intent(this, SagaInCallActivity::class.java))
            }
            "org.saga.TEST_GROUP_HANGUP_PARTICIPANT" -> {
                // DEBUG-ONLY (FLAG_DEBUGGABLE): hang up ONE participant; the group keeps running.
                // Extras: contact_name (or lookup_key), conference (optional, informational).
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_GROUP_HANGUP_PARTICIPANT refused — not a debuggable build")
                    return
                }
                val who = intent.getStringExtra("contact_name")
                    ?: intent.getStringExtra("lookup_key")
                    ?: return
                val ok = org.saga.telecom.GroupCallCoordinator.hangupParticipantByLookup(this, who)
                Log.i(TAG, "E2E group hangup participant=[$who] ok=[$ok]")
            }
            "org.saga.TEST_FORCE_IROH_SIM_PATH" -> {
                // DEBUG-ONLY: bypass enabled-account gate to measure SIM substitution / modem dial.
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_FORCE_IROH_SIM_PATH refused — not a debuggable build")
                    return
                }
                val name = intent.getStringExtra("contact_name") ?: "bob"
                val ssp = intent.getStringExtra("ssp")
                val target = DialTargetResolver.fromInput(this, name, preferCellular = false)
                if (target !is DialTarget.Iroh) {
                    Log.e(TAG, "TEST_FORCE_IROH_SIM_PATH requires Iroh contact, got [$target]")
                    return
                }
                Log.e(TAG, "TEST_FORCE_IROH_SIM_PATH contact=[$name] ssp=[$ssp]")
                SagaDialer.placeIrohCallForceSimPathForDebug(
                    this,
                    target.peerId,
                    target.lookupKey,
                    target.contactName ?: target.lookupKey,
                    sspOverride = ssp
                )
            }
            "org.saga.TEST_IROH_CALL" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_IROH_CALL refused — not a debuggable build")
                    return
                }
                Log.e(TAG, "TEST_IROH_CALL rejected — use TEST_CONTACT_CALL with contact_name (bob/alice/thor)")
                Toast.makeText(this, R.string.invalid_dial_target, Toast.LENGTH_SHORT).show()
            }
            "org.saga.TEST_RELAY_QUERY" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_RELAY_QUERY refused — not a debuggable build")
                    return
                }
                val ready = IrohRelayStatus.pollReady()
                IrohRelayStatus.writeStatus(applicationContext, ready)
                Log.i(TAG, "E2E relay query ready=[$ready]")
            }
            "org.saga.TEST_MIDCALL_REHANDSHAKE" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_MIDCALL_REHANDSHAKE refused — not a debuggable build")
                    return
                }
                val lookupKey = intent.getStringExtra("lookup_key")
                    ?: intent.getStringExtra("peer_id")
                    ?: return
                val callId = intent.getStringExtra("call_id")
                val outcome = MidCallRehandshakeOutcome.fromIntentExtras(
                    succeed = intent.getBooleanExtra("succeed", false),
                    failureMode = intent.getStringExtra("failure_mode")
                )
                SagaMidCallRehandshakeController.trigger(this, callId, lookupKey, outcome)
            }
            "org.saga.TEST_PEER_DISGUISE" -> {
                // DEBUG-ONLY: simulate peer §16 disguise chip (does not touch padlock).
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_PEER_DISGUISE refused — not a debuggable build")
                    return
                }
                val session = intent.getStringExtra("session_id") ?: "test-iroh-1"
                val active = intent.getBooleanExtra("active", true)
                org.saga.audio.VoiceDisguiseStore.onPeerDisguise(session, active)
                Log.i(TAG, "TEST_PEER_DISGUISE session=[$session] active=$active")
            }
            "org.saga.TEST_PCM_INJECT" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_PCM_INJECT refused — not a debuggable build")
                    return
                }
                val path = intent.getStringExtra("path")
                org.saga.audio.SagaPcmDebug.setInjectWav(path)
                Log.i(TAG, "TEST_PCM_INJECT path=[$path]")
            }
            "org.saga.TEST_PCM_DUMP" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_PCM_DUMP refused — not a debuggable build")
                    return
                }
                val path = intent.getStringExtra("path")
                org.saga.audio.SagaPcmDebug.setDumpPath(path)
                Log.i(TAG, "TEST_PCM_DUMP path=[$path]")
            }
            "org.saga.TEST_PCM_DUMP_FINISH" -> {
                if (!isDebuggableBuild()) {
                    Log.e(TAG, "TEST_PCM_DUMP_FINISH refused — not a debuggable build")
                    return
                }
                org.saga.audio.SagaPcmDebug.finishDump()
                Log.i(TAG, "TEST_PCM_DUMP_FINISH")
            }
            Intent.ACTION_DIAL, Intent.ACTION_VIEW, Intent.ACTION_CALL -> {
                val data = intent.data
                if (data?.scheme == "content") {
                    selectedContactUri = data
                }
                val target = DialTargetResolver.fromIntent(this, intent)
                val display = resolveContactDisplayName(data)
                    ?: data?.schemeSpecificPart?.removePrefix("//")
                    ?: intent.getStringExtra(Intent.EXTRA_PHONE_NUMBER)
                if (!display.isNullOrBlank()) {
                    binding.peerIdInput.setText(display)
                }
                if (target != null && (autoPlace || intent.action == Intent.ACTION_CALL)) {
                    placeResolved(target)
                }
            }
        }
    }

    private fun isDebuggableBuild(): Boolean =
        (applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0

    private fun openMessageForInput() {
        val raw = binding.peerIdInput.text?.toString()?.trim().orEmpty()
        val target = DialTargetResolver.fromInput(this, raw, preferCellular = false)
        // Iroh msg ALPN expects digit peer labels / EndpointIds, not E.164 with '+'.
        val peer = when (target) {
            is DialTarget.Iroh -> target.peerId.raw
            is DialTarget.Cellular -> target.number.filter { it.isDigit() }
            else -> raw.filter { it.isDigit() }.ifBlank { raw }
        }
        if (peer.isBlank()) {
            Toast.makeText(this, R.string.invalid_dial_target, Toast.LENGTH_SHORT).show()
            return
        }
        if (!org.saga.iroh.ContactKeyStore.hasResolvableKey(this, peer)) {
            Toast.makeText(this, R.string.msg_no_key, Toast.LENGTH_LONG).show()
            return
        }
        startActivity(
            Intent(this, org.saga.msg.ChatActivity::class.java)
                .putExtra(org.saga.msg.ChatActivity.EXTRA_PEER, peer)
        )
    }

    private fun placeFromInput(preferCellular: Boolean = false) {
        if (!isDialerRoleHeld()) {
            Toast.makeText(this, R.string.need_dialer_role, Toast.LENGTH_SHORT).show()
            requestDefaultDialerRole()
            return
        }
        placeFromInputForDebug(preferCellular = preferCellular)
    }

    /** Debug/e2e path: resolve + place without requiring ROLE_DIALER (PA still required). */
    private fun placeFromInputForDebug(preferCellular: Boolean = false) {
        val raw = binding.peerIdInput.text?.toString().orEmpty()
        val target = selectedContactUri?.let {
            DialTargetResolver.fromContactUri(this, it, preferCellular = preferCellular)
        } ?: DialTargetResolver.fromInput(this, raw, preferCellular)
        if (target == null) {
            Log.w(TAG, "Dial rejected — could not resolve input [$raw] uri=[$selectedContactUri]")
            Toast.makeText(this, R.string.invalid_dial_target, Toast.LENGTH_SHORT).show()
            return
        }
        placeResolved(target)
    }

    private fun placeResolved(target: DialTarget) {
        Log.i(TAG, "Placing call target=[$target] addCallMode=[$addCallMode]")
        // Add-call from InCall: prefer cellular second leg unless Iroh was chosen.
        val effective = if (addCallMode && target is DialTarget.Iroh) {
            // Keep Iroh if user tapped Call (encrypted); unencrypted button already prefers cellular.
            target
        } else {
            target
        }
        SagaDialer.placeCall(this, effective)
        if (addCallMode) {
            // Return to the single InCall composition after placing the second call.
            finish()
        }
    }

    private fun launchContactPicker() {
        if (ContextCompat.checkSelfPermission(this, android.Manifest.permission.READ_CONTACTS)
            == android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            pickContact.launch(null)
        } else {
            requestContactsPermission.launch(android.Manifest.permission.READ_CONTACTS)
        }
    }

    private fun resolveContactDisplayName(contactUri: Uri?): String? {
        if (contactUri == null) return null
        val cursor = contentResolver.query(
            contactUri,
            arrayOf(ContactsContract.Contacts.DISPLAY_NAME),
            null,
            null,
            null
        ) ?: return null
        cursor.use {
            if (!it.moveToFirst()) return null
            return it.getString(0)
        }
    }

    private fun requestDefaultDialerRole() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            Toast.makeText(this, R.string.dialer_role_denied, Toast.LENGTH_SHORT).show()
            return
        }
        val roleManager = getSystemService(RoleManager::class.java)
        if (roleManager.isRoleAvailable(RoleManager.ROLE_DIALER) &&
            !roleManager.isRoleHeld(RoleManager.ROLE_DIALER)
        ) {
            requestDialerRole.launch(roleManager.createRequestRoleIntent(RoleManager.ROLE_DIALER))
        }
    }

    private fun isDialerRoleHeld(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return true
        if (getSystemService(RoleManager::class.java).isRoleHeld(RoleManager.ROLE_DIALER)) {
            return true
        }
        // Samsung One UI: RoleManager grant via adb often fails; Settings may still
        // record the preferred dialer after settings put / user confirmation.
        val preferred = android.provider.Settings.Secure.getString(
            contentResolver,
            "dialer_default_application",
        )
        return preferred == packageName
    }

    private fun updateRoleStatus() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            binding.roleStatus.setText(R.string.dialer_role_denied)
            return
        }
        val held = isDialerRoleHeld()
        binding.roleStatus.setText(
            if (held) R.string.dialer_role_granted else R.string.dialer_role_denied
        )
    }
}

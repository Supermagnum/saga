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
    }

    private lateinit var binding: ActivityMainBinding
    private var selectedContactUri: Uri? = null

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
        binding.phoneAccountBanner.setOnClickListener {
            startActivity(Intent(this, PhoneAccountOnboardingActivity::class.java))
        }
        updateRoleStatus()
        handleIncomingIntent(intent, autoPlace = intent.action == Intent.ACTION_CALL)
    }

    override fun onResume() {
        super.onResume()
        refreshPhoneAccountUi(launchOnboardingIfNeeded = true)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (intent.action == SagaPhoneAccountRegistrar.ACTION_SHOW_ONBOARDING) {
            startActivity(Intent(this, PhoneAccountOnboardingActivity::class.java))
            return
        }
        handleIncomingIntent(intent, autoPlace = intent.action == Intent.ACTION_CALL)
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
                val lookupKey = intent.getStringExtra("lookup_key") ?: return
                Log.i(TAG, "E2E answer incoming lookup=[$lookupKey]")
                org.saga.iroh.IrohDialManager.get(this).onIncomingAnswered(lookupKey)
            }
            "org.saga.TEST_CONTACT_CALL" -> {
                val name = intent.getStringExtra("contact_name") ?: return
                val preferCellular = intent.getBooleanExtra("force_unencrypted", false)
                selectedContactUri = null
                binding.peerIdInput.setText(name)
                Log.i(
                    TAG,
                    "E2E contact dial request name=[$name] force_unencrypted=[$preferCellular]"
                )
                placeFromInput(preferCellular = preferCellular)
            }
            "org.saga.TEST_IROH_CALL" -> {
                Log.e(TAG, "TEST_IROH_CALL rejected — use TEST_CONTACT_CALL with contact_name (bob/alice/thor)")
                Toast.makeText(this, R.string.invalid_dial_target, Toast.LENGTH_SHORT).show()
            }
            "org.saga.TEST_RELAY_QUERY" -> {
                val ready = IrohRelayStatus.pollReady()
                IrohRelayStatus.writeStatus(applicationContext, ready)
                Log.i(TAG, "E2E relay query ready=[$ready]")
            }
            "org.saga.TEST_MIDCALL_REHANDSHAKE" -> {
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

    private fun placeFromInput(preferCellular: Boolean = false) {
        if (!isDialerRoleHeld()) {
            Toast.makeText(this, R.string.need_dialer_role, Toast.LENGTH_SHORT).show()
            requestDefaultDialerRole()
            return
        }
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
        Log.i(TAG, "Placing call target=[$target]")
        SagaDialer.placeCall(this, target)
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
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return false
        return getSystemService(RoleManager::class.java).isRoleHeld(RoleManager.ROLE_DIALER)
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

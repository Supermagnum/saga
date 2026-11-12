package org.saga.ui

import android.os.Bundle
import android.util.Log
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import org.saga.R
import org.saga.databinding.ActivityPhoneAccountOnboardingBinding
import org.saga.telecom.SagaPhoneAccountRegistrar

/**
 * One-shot explanation that Saga's Calling account must be enabled before Iroh calls.
 * Opens TelecomManager.ACTION_CHANGE_PHONE_ACCOUNTS; finishes when the account is enabled.
 */
class PhoneAccountOnboardingActivity : AppCompatActivity() {
    companion object {
        private const val TAG = "[Saga Phone Account Onboarding]"
    }

    private lateinit var binding: ActivityPhoneAccountOnboardingBinding

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityPhoneAccountOnboardingBinding.inflate(layoutInflater)
        setContentView(binding.root)
        SagaPhoneAccountRegistrar.register(this)
        binding.openCallingAccounts.setOnClickListener { openCallingAccounts() }
        Log.i(TAG, "Onboarding screen shown")
    }

    override fun onResume() {
        super.onResume()
        if (SagaPhoneAccountRegistrar.isAccountEnabled(this)) {
            Log.i(TAG, "PhoneAccount enabled — leaving onboarding")
            Toast.makeText(this, R.string.phone_account_enabled, Toast.LENGTH_SHORT).show()
            finish()
            return
        }
        binding.onboardingStatus.setText(R.string.phone_account_waiting)
    }

    private fun openCallingAccounts() {
        try {
            startActivity(SagaPhoneAccountRegistrar.changePhoneAccountsIntent())
            Log.i(TAG, "Opened ACTION_CHANGE_PHONE_ACCOUNTS")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to open calling accounts: ${e.message}")
            Toast.makeText(this, R.string.phone_account_settings_failed, Toast.LENGTH_LONG).show()
        }
    }
}

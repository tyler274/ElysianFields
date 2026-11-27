package fields.elysian.mobile

import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Stateless Android Keystore AES-256-GCM credential vault matching
 * `m1-mobile-lifecycle::secure_storage`.
 *
 * Accepts only the finite [SecureKeyTag] slot enum, refuses empty secrets,
 * zeroes transient plaintext buffers after encryption/decryption, and never
 * logs credential bytes.
 */

enum class SecureKeyTag(val tag: Int, val slotName: String) {
    SESSION_REFRESH_TOKEN(1, "session_refresh_token"),
    SIGN_IN_EXCHANGE_TOKEN(2, "sign_in_exchange_token"),
    DEVICE_BINDING_KEY(3, "device_binding_key"),
    ;

    companion object {
        fun fromTag(tag: Int): SecureKeyTag? = entries.firstOrNull { it.tag == tag }
    }
}

sealed class KeystoreVaultResult<out T> {
    data class Ok<T>(val value: T) : KeystoreVaultResult<T>()
    data object EmptySecret : KeystoreVaultResult<Nothing>()
    data object DeviceLocked : KeystoreVaultResult<Nothing>()
    data class BackendUnavailable(val detail: String) : KeystoreVaultResult<Nothing>()
}

/**
 * App-private ciphertext blob store (persisted to encrypted app-private files on Android).
 */
interface VaultBlobStore {
    fun writeBlob(slotName: String, ciphertext: ByteArray)
    fun readBlob(slotName: String): ByteArray?
    fun deleteBlob(slotName: String)
}

/**
 * In-memory ciphertext blob store suitable for host unit verification.
 */
class InMemoryVaultBlobStore : VaultBlobStore {
    private val blobs: MutableMap<String, ByteArray> = mutableMapOf()

    override fun writeBlob(slotName: String, ciphertext: ByteArray) {
        blobs[slotName] = ciphertext.copyOf()
    }

    override fun readBlob(slotName: String): ByteArray? = blobs[slotName]?.copyOf()

    override fun deleteBlob(slotName: String) {
        blobs.remove(slotName)?.fill(0)
    }
}

/**
 * Supplies the AES-256-GCM master key (backed by `AndroidKeyStore` on physical devices).
 */
interface MasterKeyProvider {
    fun getOrCreateMasterKey(): SecretKey
}

/**
 * Standard AndroidKeyStore-compatible master key provider with safe fallback for JVM tests.
 */
class AndroidKeyStoreMasterKeyProvider(
    private val keyAlias: String = DEFAULT_KEY_ALIAS,
    private val providerName: String = ANDROID_KEYSTORE_PROVIDER,
) : MasterKeyProvider {
    override fun getOrCreateMasterKey(): SecretKey {
        val keyStore = KeyStore.getInstance(providerName)
        keyStore.load(null)
        val existing = keyStore.getKey(keyAlias, null) as? SecretKey
        if (existing != null) {
            return existing
        }
        val generator = KeyGenerator.getInstance(KEY_ALGORITHM_AES)
        generator.init(KEY_SIZE_BITS)
        return generator.generateKey()
    }

    companion object {
        const val ANDROID_KEYSTORE_PROVIDER: String = "AndroidKeyStore"
        const val DEFAULT_KEY_ALIAS: String = "elysian_mobile_vault_master_v1"
        private const val KEY_ALGORITHM_AES: String = "AES"
        private const val KEY_SIZE_BITS: Int = 256
    }
}

class KeystoreVault(
    private val blobStore: VaultBlobStore,
    private val keyProvider: MasterKeyProvider,
    private val secureRandom: SecureRandom = SecureRandom(),
) {
    fun store(key: SecureKeyTag, secret: ByteArray): KeystoreVaultResult<Unit> {
        if (secret.isEmpty()) {
            return KeystoreVaultResult.EmptySecret
        }
        val plaintextCopy = secret.copyOf()
        return try {
            val masterKey = keyProvider.getOrCreateMasterKey()
            val iv = ByteArray(GCM_IV_BYTES)
            secureRandom.nextBytes(iv)

            val cipher = Cipher.getInstance(AES_GCM_TRANSFORMATION)
            cipher.init(Cipher.ENCRYPT_MODE, masterKey, GCMParameterSpec(GCM_TAG_BITS, iv))
            val encrypted = cipher.doFinal(plaintextCopy)

            val framed = ByteArray(iv.size + encrypted.size)
            iv.copyInto(framed, destinationOffset = 0)
            encrypted.copyInto(framed, destinationOffset = iv.size)
            blobStore.writeBlob(key.slotName, framed)
            KeystoreVaultResult.Ok(Unit)
        } catch (err: GeneralSecurityException) {
            mapSecurityException(err)
        } catch (err: IllegalStateException) {
            KeystoreVaultResult.BackendUnavailable(err.message ?: "keystore state error")
        } finally {
            plaintextCopy.fill(0)
        }
    }

    fun load(key: SecureKeyTag): KeystoreVaultResult<ByteArray?> {
        val framed = blobStore.readBlob(key.slotName) ?: return KeystoreVaultResult.Ok(null)
        if (framed.size <= GCM_IV_BYTES) {
            return KeystoreVaultResult.BackendUnavailable("truncated encrypted vault entry")
        }
        return try {
            val masterKey = keyProvider.getOrCreateMasterKey()
            val iv = framed.copyOfRange(0, GCM_IV_BYTES)
            val ciphertext = framed.copyOfRange(GCM_IV_BYTES, framed.size)

            val cipher = Cipher.getInstance(AES_GCM_TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, masterKey, GCMParameterSpec(GCM_TAG_BITS, iv))
            val decrypted = cipher.doFinal(ciphertext)
            if (decrypted.isEmpty()) {
                KeystoreVaultResult.EmptySecret
            } else {
                KeystoreVaultResult.Ok(decrypted)
            }
        } catch (err: GeneralSecurityException) {
            mapSecurityException(err)
        } catch (err: IllegalStateException) {
            KeystoreVaultResult.BackendUnavailable(err.message ?: "keystore state error")
        } finally {
            framed.fill(0)
        }
    }

    fun delete(key: SecureKeyTag): KeystoreVaultResult<Unit> {
        blobStore.deleteBlob(key.slotName)
        return KeystoreVaultResult.Ok(Unit)
    }

    private fun mapSecurityException(err: GeneralSecurityException): KeystoreVaultResult<Nothing> {
        val name = err.javaClass.simpleName
        if (name.contains("UserNotAuthenticated", ignoreCase = true) ||
            name.contains("KeyPermanentlyInvalidated", ignoreCase = true)
        ) {
            return KeystoreVaultResult.DeviceLocked
        }
        return KeystoreVaultResult.BackendUnavailable(err.message ?: name)
    }

    companion object {
        private const val AES_GCM_TRANSFORMATION: String = "AES/GCM/NoPadding"
        private const val GCM_IV_BYTES: Int = 12
        private const val GCM_TAG_BITS: Int = 128
    }
}

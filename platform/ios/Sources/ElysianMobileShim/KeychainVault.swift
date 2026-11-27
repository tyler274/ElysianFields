import Foundation
#if canImport(Security)
import Security
#endif

public enum KeychainVaultError: Error, Equatable {
    case emptySecret
    case deviceLocked
    case backendUnavailable(status: Int32)
}

/// Stateless iOS Keychain Services wrapper matching `m1-mobile-lifecycle::secure_storage`.
///
/// Accepts only finite `SecureKeyTag` slots, binds entries to
/// `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, maps locked-device Keychain status
/// (`errSecInteractionNotAllowed`) to `.deviceLocked` instead of returning empty data,
/// scrubs transient byte copies before returning, and never logs secret bytes.
public final class KeychainVault {
    public let serviceIdentifier: String

    public init(serviceIdentifier: String = "fields.elysian.mobile.vault.v1") {
        self.serviceIdentifier = serviceIdentifier
    }

    public func store(key: SecureKeyTag, secret: [UInt8]) throws {
        guard !secret.isEmpty else {
            throw KeychainVaultError.emptySecret
        }

        #if canImport(Security)
        var transientBytes = secret
        defer {
            for index in transientBytes.indices {
                transientBytes[index] = 0
            }
        }
        let secretData = Data(transientBytes)

        let baseQuery: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceIdentifier,
            kSecAttrAccount as String: key.accountIdentifier,
        ]
        var addQuery = baseQuery
        addQuery[kSecValueData as String] = secretData
        addQuery[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly

        let addStatus = SecItemAdd(addQuery as CFDictionary, nil)
        if addStatus == errSecSuccess {
            return
        }
        if addStatus == errSecDuplicateItem {
            let updateFields: [String: Any] = [
                kSecValueData as String: secretData,
                kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            ]
            let updateStatus = SecItemUpdate(baseQuery as CFDictionary, updateFields as CFDictionary)
            if updateStatus == errSecSuccess {
                return
            }
            throw Self.mapStatus(updateStatus)
        }
        throw Self.mapStatus(addStatus)
        #else
        throw KeychainVaultError.backendUnavailable(status: -1)
        #endif
    }

    public func load(key: SecureKeyTag) throws -> [UInt8]? {
        #if canImport(Security)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceIdentifier,
            kSecAttrAccount as String: key.accountIdentifier,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw Self.mapStatus(status)
        }
        guard let data = item as? Data else {
            throw KeychainVaultError.backendUnavailable(status: status)
        }
        let bytes = [UInt8](data)
        guard !bytes.isEmpty else {
            throw KeychainVaultError.emptySecret
        }
        return bytes
        #else
        throw KeychainVaultError.backendUnavailable(status: -1)
        #endif
    }

    public func delete(key: SecureKeyTag) throws {
        #if canImport(Security)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceIdentifier,
            kSecAttrAccount as String: key.accountIdentifier,
        ]
        let status = SecItemDelete(query as CFDictionary)
        if status == errSecSuccess || status == errSecItemNotFound {
            return
        }
        throw Self.mapStatus(status)
        #else
        throw KeychainVaultError.backendUnavailable(status: -1)
        #endif
    }

    #if canImport(Security)
    private static func mapStatus(_ status: OSStatus) -> KeychainVaultError {
        if status == errSecInteractionNotAllowed {
            return .deviceLocked
        }
        return .backendUnavailable(status: Int32(status))
    }
    #endif
}

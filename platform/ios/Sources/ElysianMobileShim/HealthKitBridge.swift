import Foundation
#if canImport(HealthKit) && os(iOS)
import HealthKit
#endif

/// Typed numeric sample payload returned from `HKHealthStore` to the Rust sync worker.
public struct HealthSampleWire: Equatable {
    public let metric: HealthMetricTag
    public let primaryValue: Int64
    public let secondaryValue: Int64
    public let recordedAtEpochSecs: UInt64

    public init(
        metric: HealthMetricTag,
        primaryValue: Int64,
        secondaryValue: Int64 = 0,
        recordedAtEpochSecs: UInt64
    ) {
        self.metric = metric
        self.primaryValue = primaryValue
        self.secondaryValue = secondaryValue
        self.recordedAtEpochSecs = recordedAtEpochSecs
    }
}

public enum HealthKitBridgeError: Error, Equatable {
    case permissionRequired(metric: HealthMetricTag, state: HealthPermissionStateTag)
    case platformUnavailable
    case adapterFailure(detail: String)
}

/// Stateless iOS HealthKit adapter shim matching `m1-mobile-lifecycle::health`.
///
/// Respects Apple's read-privacy model (never loops prompts once authorization has been
/// presented for a metric; maps empty authorized queries to `nil` so Rust presents
/// `HealthUiOutcome::AuthorizedNoData` with a non-blocking Settings recovery hint).
public final class HealthKitBridge {
    private let lock = NSLock()
    private var requestedMetrics: Set<HealthMetricTag> = []

    #if canImport(HealthKit) && os(iOS)
    private let store: HKHealthStore?
    #endif

    public init() {
        #if canImport(HealthKit) && os(iOS)
        self.store = HKHealthStore.isHealthDataAvailable() ? HKHealthStore() : nil
        #endif
    }

    public func isAvailable() -> Bool {
        #if canImport(HealthKit) && os(iOS)
        return store != nil
        #else
        return false
        #endif
    }

    public func permissionState(for metric: HealthMetricTag) -> HealthPermissionStateTag {
        guard isAvailable() else {
            return .unavailable
        }
        lock.lock()
        let alreadyRequested = requestedMetrics.contains(metric)
        lock.unlock()
        return alreadyRequested ? .granted : .notDetermined
    }

    public func requestPermission(
        for metric: HealthMetricTag,
        completion: @escaping (Result<HealthPermissionStateTag, HealthKitBridgeError>) -> Void
    ) {
        guard isAvailable() else {
            completion(.success(.unavailable))
            return
        }

        let current = permissionState(for: metric)
        if current != .notDetermined {
            completion(.success(current))
            return
        }

        #if canImport(HealthKit) && os(iOS)
        guard let store = self.store, let sampleType = Self.objectType(for: metric) else {
            completion(.success(.unavailable))
            return
        }
        store.requestAuthorization(toShare: nil, read: Set([sampleType])) { [weak self] success, error in
            if let error {
                completion(.failure(.adapterFailure(detail: error.localizedDescription)))
                return
            }
            self?.lock.lock()
            self?.requestedMetrics.insert(metric)
            self?.lock.unlock()
            completion(.success(success ? .granted : .denied))
        }
        #else
        completion(.success(.unavailable))
        #endif
    }

    public func readLatestSample(
        for metric: HealthMetricTag,
        completion: @escaping (Result<HealthSampleWire?, HealthKitBridgeError>) -> Void
    ) {
        let state = permissionState(for: metric)
        guard state == .granted else {
            if state == .unavailable {
                completion(.failure(.platformUnavailable))
            } else {
                completion(.failure(.permissionRequired(metric: metric, state: state)))
            }
            return
        }

        #if canImport(HealthKit) && os(iOS)
        guard let store = self.store, let sampleType = Self.sampleType(for: metric) else {
            completion(.failure(.platformUnavailable))
            return
        }
        let sort = NSSortDescriptor(key: HKSampleSortIdentifierEndDate, ascending: false)
        let query = HKSampleQuery(
            sampleType: sampleType,
            predicate: nil,
            limit: 1,
            sortDescriptors: [sort]
        ) { _, samples, error in
            if let error {
                completion(.failure(.adapterFailure(detail: error.localizedDescription)))
                return
            }
            guard let latest = samples?.first else {
                completion(.success(nil))
                return
            }
            completion(.success(Self.convertSample(latest, metric: metric)))
        }
        store.execute(query)
        #else
        completion(.failure(.platformUnavailable))
        #endif
    }

    #if canImport(HealthKit) && os(iOS)
    private static func objectType(for metric: HealthMetricTag) -> HKObjectType? {
        sampleType(for: metric)
    }

    private static func sampleType(for metric: HealthMetricTag) -> HKSampleType? {
        switch metric {
        case .heartRateBpm:
            return HKObjectType.quantityType(forIdentifier: .heartRate)
        case .stepsCount:
            return HKObjectType.quantityType(forIdentifier: .stepCount)
        case .bloodPressureMmHg:
            return HKObjectType.quantityType(forIdentifier: .bloodPressureSystolic)
        case .sleepDurationMinutes:
            return HKObjectType.categoryType(forIdentifier: .sleepAnalysis)
        }
    }

    private static func convertSample(_ sample: HKSample, metric: HealthMetricTag) -> HealthSampleWire? {
        let epochSecs = UInt64(max(0, sample.endDate.timeIntervalSince1970))
        switch metric {
        case .heartRateBpm:
            guard let qty = sample as? HKQuantitySample else { return nil }
            let bpm = qty.quantity.doubleValue(for: HKUnit.count().unitDivided(by: .minute()))
            return HealthSampleWire(metric: metric, primaryValue: Int64(bpm.rounded()), recordedAtEpochSecs: epochSecs)
        case .stepsCount:
            guard let qty = sample as? HKQuantitySample else { return nil }
            let count = qty.quantity.doubleValue(for: HKUnit.count())
            return HealthSampleWire(metric: metric, primaryValue: Int64(count.rounded()), recordedAtEpochSecs: epochSecs)
        case .bloodPressureMmHg:
            guard let qty = sample as? HKQuantitySample else { return nil }
            let systolic = qty.quantity.doubleValue(for: HKUnit.millimeterOfMercury())
            return HealthSampleWire(metric: metric, primaryValue: Int64(systolic.rounded()), recordedAtEpochSecs: epochSecs)
        case .sleepDurationMinutes:
            let mins = max(0, sample.endDate.timeIntervalSince(sample.startDate) / 60.0)
            return HealthSampleWire(metric: metric, primaryValue: Int64(mins.rounded()), recordedAtEpochSecs: epochSecs)
        }
    }
    #endif
}

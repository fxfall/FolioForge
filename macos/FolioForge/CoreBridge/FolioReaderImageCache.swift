import AppKit

private final class CachedReaderImage: NSObject {
    let displayImage: NSImage
    let raster: CGImage

    init(displayImage: NSImage, raster: CGImage) {
        self.displayImage = displayImage
        self.raster = raster
    }
}

/// Session-scoped AppKit image cache for Reader resources. It stores no source
/// paths or semantic state; Folio IR and Reader remain the display authority.
@MainActor
final class FolioReaderImageCache {
    private static let maximumDecodedBytes = 128 * 1024 * 1024
    private let images = NSCache<NSString, CachedReaderImage>()

    init() {
        images.countLimit = 4
        images.totalCostLimit = Self.maximumDecodedBytes
    }

    func image(data: Data, sessionID: String, resourceID: UInt32) -> NSImage? {
        let key = "\(sessionID):\(resourceID)" as NSString
        if let cached = images.object(forKey: key) {
            return cached.displayImage
        }

        guard let encodedImage = NSImage(data: data) else { return nil }
        var proposedRect = CGRect(origin: .zero, size: encodedImage.size)
        guard let decodedImage = encodedImage.cgImage(
            forProposedRect: &proposedRect,
            context: nil,
            hints: nil
        ) else {
            return encodedImage
        }

        let (decodedBytes, overflow) = decodedImage.bytesPerRow
            .multipliedReportingOverflow(by: decodedImage.height)
        guard !overflow, decodedBytes <= Self.maximumDecodedBytes else {
            return encodedImage
        }
        images.setObject(
            CachedReaderImage(displayImage: encodedImage, raster: decodedImage),
            forKey: key,
            cost: decodedBytes
        )
        return encodedImage
    }

    func removeAll() {
        images.removeAllObjects()
    }
}

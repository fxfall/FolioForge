import AppKit
import SwiftUI

struct FolioForgeLogo: View {
    private let image = Bundle.module
        .url(forResource: "folioforge-logo", withExtension: "png")
        .flatMap { NSImage(contentsOf: $0) }

    var body: some View {
        if let image {
            Image(nsImage: image)
                .resizable()
                .scaledToFit()
        } else {
            Image(folioSymbol: .bookFilled)
                .resizable()
                .scaledToFit()
                .foregroundStyle(.secondary)
        }
    }
}

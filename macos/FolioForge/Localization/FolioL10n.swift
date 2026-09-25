import Foundation

/// Resolves semantic UI keys from this SwiftPM target's Apple String Catalog.
/// Core identifiers and technical details must not be passed here.
enum FolioL10n {
    private static let languagePreferenceKey = "folioforge.ui.language"

    static func locale(for preference: String) -> Locale {
        switch preference {
        case "en":
            Locale(identifier: "en")
        case "zh-Hans":
            Locale(identifier: "zh-Hans")
        default:
            Locale.current.identifier.lowercased().hasPrefix("zh")
                ? Locale(identifier: "zh-Hans")
                : Locale(identifier: "en")
        }
    }

    private static var selectedLocale: Locale {
        locale(for: UserDefaults.standard.string(forKey: languagePreferenceKey) ?? "system")
    }

    static func string(
        _ key: StaticString,
        default fallback: String.LocalizationValue
    ) -> String {
        String(
            localized: LocalizedStringResource(
                key,
                defaultValue: fallback,
                locale: selectedLocale,
                bundle: .module
            )
        )
    }

    static func format(
        _ key: StaticString,
        default fallback: String.LocalizationValue,
        _ arguments: CVarArg...
    ) -> String {
        let resource = LocalizedStringResource(
            key,
            defaultValue: fallback,
            locale: selectedLocale,
            bundle: .module
        )
        return String(
            format: String(localized: resource),
            locale: selectedLocale,
            arguments: arguments
        )
    }
}

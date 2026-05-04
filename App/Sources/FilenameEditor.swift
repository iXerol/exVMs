import AppKit
import SwiftUI

struct FilenameEditor: View {
    @Binding var recipe: FilenameRecipe
    @State private var editorBridge = FilenameTextEditorBridge()

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                ForEach(FilenameToken.allCases) { token in
                    FilenameTokenChip(token: token) {
                        editorBridge.insert(token: token)
                    }
                }
                Spacer()
            }

            FilenameTextEditor(recipe: $recipe, bridge: editorBridge)
                .frame(minWidth: 420, maxWidth: .infinity, minHeight: 44, idealHeight: 44, maxHeight: 44)
                .background(.quaternary.opacity(0.45))
                .clipShape(RoundedRectangle(cornerRadius: 8))
                .overlay {
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(Color.secondary.opacity(0.35), lineWidth: 1)
                }

            HStack(spacing: 8) {
                Text("ui.filename.example.label")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                ScrollView(.horizontal) {
                    Text(sampleFilename)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .font(.body.monospaced())
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(.green.opacity(0.10))
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                        .accessibilityLabel("accessibility.filename.example")
                        .accessibilityValue(sampleFilename)
                }.scrollIndicators(.hidden)
            }
        }
        .accessibilityElement(children: .contain)
    }

    private var sampleFilename: String {
        recipe.sampleFilename()
    }
}

@MainActor
final class FilenameTextEditorBridge {
    weak var textView: NSTextView?

    func insert(token: FilenameToken) {
        guard let textView else {
            return
        }

        let boundedIndex = min(textView.selectedRange().location, textView.textStorage?.length ?? 0)
        textView.window?.makeFirstResponder(textView)
        textView.textStorage?.replaceCharacters(
            in: NSRange(location: boundedIndex, length: 0),
            with: FilenameAttributedText.tokenAttachment(token)
        )
        textView.setSelectedRange(NSRange(location: boundedIndex + 1, length: 0))
        textView.typingAttributes = FilenameAttributedText.literalTypingAttributes
        textView.didChangeText()
    }
}

private struct FilenameTextEditor: NSViewRepresentable {
    @Binding var recipe: FilenameRecipe
    var bridge: FilenameTextEditorBridge

    func makeCoordinator() -> Coordinator {
        Coordinator(recipe: $recipe)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.drawsBackground = false
        scrollView.hasHorizontalScroller = true
        scrollView.hasVerticalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder

        let textView = FilenameNSTextView()
        textView.delegate = context.coordinator
        textView.drawsBackground = false
        textView.isRichText = true
        textView.importsGraphics = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.allowsUndo = true
        textView.font = .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        textView.textContainerInset = NSSize(width: 10, height: 10)
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.widthTracksTextView = false
        textView.textContainer?.containerSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: 24)
        textView.isHorizontallyResizable = true
        textView.isVerticallyResizable = false
        textView.minSize = NSSize(width: 0, height: 44)
        textView.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: 44)
        textView.frame = NSRect(x: 0, y: 0, width: 1_000, height: 44)
        textView.setAccessibilityLabel(ExVMsStrings.Accessibility.Filename.pattern)
        textView.setAccessibilityHelp(ExVMsStrings.Accessibility.Filename.Pattern.help)
        textView.filenameAccessibilityValue = recipe.accessibilityDescription

        scrollView.documentView = textView
        bridge.textView = textView
        context.coordinator.textView = textView
        context.coordinator.apply(recipe)
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.recipe = $recipe
        guard let textView = scrollView.documentView as? NSTextView else {
            return
        }
        bridge.textView = textView
        context.coordinator.textView = textView
        textView.frame.size = NSSize(width: max(scrollView.contentSize.width, textView.frame.width), height: 44)
        if FilenameAttributedText.semanticKey(textView.attributedString()) != recipe.semanticKey {
            context.coordinator.apply(recipe)
        } else {
            (textView as? FilenameNSTextView)?.filenameAccessibilityValue = recipe.accessibilityDescription
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var recipe: Binding<FilenameRecipe>
        weak var textView: NSTextView?
        private var isApplying = false

        init(recipe: Binding<FilenameRecipe>) {
            self.recipe = recipe
        }

        func apply(_ recipe: FilenameRecipe) {
            guard let textView else {
                return
            }

            isApplying = true
            textView.textStorage?.setAttributedString(FilenameAttributedText.attributedString(from: recipe))
            textView.setSelectedRange(NSRange(location: textView.textStorage?.length ?? 0, length: 0))
            (textView as? FilenameNSTextView)?.filenameAccessibilityValue = recipe.accessibilityDescription
            isApplying = false
        }

        func textDidChange(_ notification: Notification) {
            guard !isApplying,
                  let textView = notification.object as? NSTextView else {
                return
            }

            textView.typingAttributes = FilenameAttributedText.literalTypingAttributes
            let nextRecipe = FilenameAttributedText.recipe(from: textView.attributedString())
            (textView as? FilenameNSTextView)?.filenameAccessibilityValue = nextRecipe.accessibilityDescription
            recipe.wrappedValue = nextRecipe
        }

        func textViewDidChangeSelection(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else {
                return
            }

            textView.typingAttributes = FilenameAttributedText.literalTypingAttributes
        }
    }
}

private final class FilenameNSTextView: NSTextView {
    nonisolated(unsafe) var filenameAccessibilityValue = ""

    override var intrinsicContentSize: NSSize {
        NSSize(width: NSView.noIntrinsicMetric, height: 44)
    }

    override func accessibilityValue() -> String? {
        filenameAccessibilityValue
    }

    override func paste(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string) else {
            return
        }
        insertText(text, replacementRange: selectedRange())
    }
}

@MainActor
private enum FilenameAttributedText {
    private static let tokenAttribute = NSAttributedString.Key("com.ixerol.exvms.filename-token")

    static let literalTypingAttributes: [NSAttributedString.Key: Any] = [
        .font: NSFont.monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular),
        .foregroundColor: NSColor.labelColor,
    ]

    static func attributedString(from recipe: FilenameRecipe) -> NSAttributedString {
        let output = NSMutableAttributedString()
        for part in recipe.parts {
            switch part {
            case .literal(_, let value):
                output.append(literal(value))
            case .token(_, let token):
                output.append(tokenAttachment(token))
            }
        }
        return output
    }

    static func recipe(from attributedString: NSAttributedString) -> FilenameRecipe {
        var parts: [FilenamePart] = []
        let fullRange = NSRange(location: 0, length: attributedString.length)

        attributedString.enumerateAttributes(in: fullRange) { attributes, range, _ in
            let text = attributedString.attributedSubstring(from: range).string
            if let token = token(from: attributes[tokenAttribute]) {
                parts.append(.token(UUID(), token))
                return
            }

            if text == String(NSTextAttachment.character) {
                return
            }
            appendLiteral(text, to: &parts)
        }

        if parts.isEmpty {
            return .default
        }
        return FilenameRecipe(parts: parts)
    }

    static func tokenAttachment(_ token: FilenameToken) -> NSAttributedString {
        let attachment = NSTextAttachment()
        attachment.image = token.capsuleImage()
        attachment.bounds = CGRect(
            x: 0,
            y: -5,
            width: token.inlineSize.width,
            height: FilenameTokenStyle.height
        )

        let output = NSMutableAttributedString(attachment: attachment)
        output.addAttribute(tokenAttribute, value: token.rawValue, range: NSRange(location: 0, length: 1))
        return output
    }

    static func semanticKey(_ attributedString: NSAttributedString) -> [String] {
        recipe(from: attributedString).semanticKey
    }

    private static func literal(_ value: String) -> NSAttributedString {
        NSAttributedString(string: value, attributes: literalTypingAttributes)
    }

    private static func appendLiteral(_ literal: String, to parts: inout [FilenamePart]) {
        guard !literal.isEmpty else {
            return
        }
        if case .literal(let id, let value) = parts.last {
            parts[parts.count - 1] = .literal(id, value + literal)
        } else {
            parts.append(.literal(UUID(), literal))
        }
    }

    private static func token(from value: Any?) -> FilenameToken? {
        if let rawValue = value as? String {
            return FilenameToken(rawValue: rawValue)
        }
        if let rawValue = value as? NSString {
            return FilenameToken(rawValue: rawValue as String)
        }
        return nil
    }

}

private struct FilenameTokenChip: View {
    var token: FilenameToken
    var onInsert: () -> Void

    var body: some View {
        Button {
            onInsert()
        } label: {
            FilenameTokenPill(token: token)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("accessibility.filename.insert_token \(token.accessibilityName)")
        .accessibilityHint("accessibility.filename.insert_token.hint")
        .help(Text("help.filename.insert_token \(token.accessibilityName)"))
    }
}

private struct FilenameTokenPill: View {
    var token: FilenameToken

    var body: some View {
        Text(token.label)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(.white)
            .frame(
                width: token.inlineSize.width,
                height: FilenameTokenStyle.height
            )
            .background {
                Capsule()
                    .fill(FilenameTokenStyle.swiftUIColor)
                    .strokeBorder(FilenameTokenStyle.swiftUIStrokeColor, lineWidth: FilenameTokenStyle.strokeWidth)
            }
    }
}

private enum FilenameTokenStyle {
    static let fontSize: CGFloat = 11
    static let fontWeight = NSFont.Weight.semibold
    static let horizontalPadding: CGFloat = 10
    static let height: CGFloat = 24
    static let strokeWidth: CGFloat = 1

    static let nsColor = NSColor.systemBlue.withAlphaComponent(0.88)
    static let nsStrokeColor = NSColor.systemGray.withAlphaComponent(0.5)
    static let swiftUIColor = Color(nsColor: nsColor)
    static let swiftUIStrokeColor = Color(nsColor: nsStrokeColor)

    static var font: NSFont {
        NSFont.systemFont(ofSize: fontSize, weight: fontWeight)
    }

    static var textAttributes: [NSAttributedString.Key: Any] {
        [
            .font: font,
            .foregroundColor: NSColor.white,
        ]
    }
}

private extension FilenameRecipe {
    var semanticKey: [String] {
        parts.map { part in
            switch part {
            case .literal(_, let value):
                "literal:\(value)"
            case .token(_, let token):
                "token:\(token.rawValue)"
            }
        }
    }
}

@MainActor
private extension FilenameToken {
    var inlineSize: CGSize {
        let textSize = label.size(withAttributes: FilenameTokenStyle.textAttributes)
        return CGSize(
            width: ceil(textSize.width + FilenameTokenStyle.horizontalPadding * 2),
            height: FilenameTokenStyle.height
        )
    }

    func capsuleImage() -> NSImage {
        let content = FilenameTokenPill(token: self)
            .frame(width: inlineSize.width, height: inlineSize.height)
        let renderer = ImageRenderer(content: content)
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        return renderer.nsImage ?? NSImage(size: NSSize(width: inlineSize.width, height: inlineSize.height))
    }
}

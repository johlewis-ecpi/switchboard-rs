import AppKit
import Foundation
import QuickLookThumbnailing
import WebKit

let maximumDocumentBytes = 40 * 1024 * 1024
let maximumImageBytes = 8 * 1024 * 1024
var scratchToRemove: URL?
var previewStage = "requesting a Quick Look thumbnail"

func describe(_ error: Error) -> String {
    var details: [String] = []
    var current: NSError? = error as NSError
    for _ in 0..<4 {
        guard let next = current else { break }
        details.append("\(next.localizedDescription) (\(next.domain) \(next.code))")
        current = next.userInfo[NSUnderlyingErrorKey] as? NSError
    }
    return details.joined(separator: "; ")
}

func fail(_ message: String) -> Never {
    if let scratch = scratchToRemove { try? FileManager.default.removeItem(at: scratch) }
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func saveImage(_ image: CGImage, to destination: URL) throws {
    let bitmap = NSBitmapImageRep(cgImage: image)
    guard let data = bitmap.representation(using: .png, properties: [:]) else {
        throw NSError(domain: "SwitchboardPreview", code: 1,
                      userInfo: [NSLocalizedDescriptionKey: "Could not encode document preview"])
    }
    try data.write(to: destination, options: .atomic)
}

func quickLook(_ source: URL, completion: @escaping (Result<CGImage, Error>) -> Void) {
    let request = QLThumbnailGenerator.Request(
        fileAt: source, size: CGSize(width: 736, height: 520), scale: 1,
        representationTypes: [.thumbnail, .lowQualityThumbnail])
    // Both completion and deadline run on the main queue, so a slow/cancelled
    // request cannot complete twice or suppress the independent fallbacks.
    var completed = false
    QLThumbnailGenerator.shared.generateBestRepresentation(for: request) { thumbnail, error in
        DispatchQueue.main.async {
            guard !completed else { return }
            completed = true
            if let thumbnail { completion(.success(thumbnail.cgImage)) }
            else {
                completion(.failure(error ?? NSError(domain: "SwitchboardPreview", code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "Quick Look returned no thumbnail"])))
            }
        }
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + 4) {
        guard !completed else { return }
        completed = true
        QLThumbnailGenerator.shared.cancel(request)
        completion(.failure(NSError(domain: "SwitchboardPreview", code: 3,
            userInfo: [NSLocalizedDescriptionKey: "Quick Look timed out"])))
    }
}

// Read through the file provider into an ordinary local file. This avoids
// asking the provider for a cloud thumbnail and never changes the original.
func readSnapshot(_ source: URL, in directory: URL) throws -> URL {
    let input = try FileHandle(forReadingFrom: source)
    defer { try? input.close() }
    var data = Data()
    while let chunk = try input.read(upToCount: min(65536, maximumDocumentBytes + 1 - data.count)), !chunk.isEmpty {
        data.append(chunk)
        if data.count > maximumDocumentBytes {
            throw NSError(domain: "SwitchboardPreview", code: 4,
                userInfo: [NSLocalizedDescriptionKey: "Document exceeds the 40 MiB fallback limit"])
        }
    }
    let copy = directory.appendingPathComponent("document").appendingPathExtension(source.pathExtension)
    try data.write(to: copy, options: .atomic)
    return copy
}

func localSnapshot(_ source: URL, in directory: URL) throws -> URL {
    // File providers prepare readable contents during coordinated access.
    // Opening their path directly can time out even when Finder lists the file.
    let coordinator = NSFileCoordinator(filePresenter: nil)
    var coordinationError: NSError?
    var result: Result<URL, Error>?
    coordinator.coordinate(readingItemAt: source, options: .withoutChanges, error: &coordinationError) { readableURL in
        result = Result { try readSnapshot(readableURL, in: directory) }
    }
    if let coordinationError { throw coordinationError }
    guard let result else {
        throw NSError(domain: "SwitchboardPreview", code: 5,
            userInfo: [NSLocalizedDescriptionKey: "The file provider did not grant readable access"])
    }
    return try result.get()
}

// Office packages can carry a saved first-page/slide thumbnail. Read only the
// named image, with a strict output bound; do not unpack the archive to disk.
func officeThumbnail(_ source: URL) -> CGImage? {
    guard ["docx", "pptx", "dotx", "potx", "ppsx"].contains(source.pathExtension.lowercased()) else { return nil }
    for name in ["docProps/thumbnail.jpeg", "docProps/thumbnail.png", "docProps/thumbnail.jpg"] {
        let process = Process()
        let pipe = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/unzip")
        process.arguments = ["-p", source.path, name]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            var data = Data()
            while let chunk = try pipe.fileHandleForReading.read(upToCount: 65536), !chunk.isEmpty {
                data.append(chunk)
                if data.count > maximumImageBytes { process.terminate(); break }
            }
            process.waitUntilExit()
            if process.terminationStatus == 0, data.count <= maximumImageBytes,
               let bitmap = NSBitmapImageRep(data: data), let image = bitmap.cgImage {
                return image
            }
        } catch { if process.isRunning { process.terminate() } }
    }
    return nil
}

// Native Word import gives a visual first-page excerpt when neither Quick Look
// nor a saved Office thumbnail is available. Complex pagination may differ.
func wordPage(_ source: URL) throws -> CGImage? {
    let ext = source.pathExtension.lowercased()
    guard ["doc", "docx", "rtf"].contains(ext) else { return nil }
    let type: NSAttributedString.DocumentType = ext == "docx" ? .officeOpenXML : ext == "doc" ? .docFormat : .rtf
    let text = try NSAttributedString(url: source, options: [.documentType: type], documentAttributes: nil)
    guard text.length > 0 else { return nil }
    let size = NSSize(width: 612, height: 792)
    let image = NSImage(size: size, flipped: true) { rect in
        NSColor.white.setFill(); rect.fill()
        NSGraphicsContext.saveGraphicsState()
        rect.clip()
        text.draw(with: rect.insetBy(dx: 42, dy: 42), options: [.usesLineFragmentOrigin, .usesFontLeading])
        NSGraphicsContext.restoreGraphicsState()
        return true
    }
    return image.cgImage(forProposedRect: nil, context: nil, hints: nil)
}

final class HTMLPreview: NSObject, WKNavigationDelegate {
    let view = WKWebView(frame: NSRect(x: 0, y: 0, width: 1100, height: 760))
    let destination: URL
    init(source: URL, destination: URL) {
        self.destination = destination
        super.init()
        view.navigationDelegate = self
        view.loadFileURL(source, allowingReadAccessTo: source.deletingLastPathComponent())
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            webView.takeSnapshot(with: nil) { image, error in
                guard let cgImage = image?.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
                    fail(error?.localizedDescription ?? "Browser rendering produced no image")
                }
                complete(cgImage)
            }
        }
    }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { fail(error.localizedDescription) }
    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { fail(error.localizedDescription) }
}

let arguments = CommandLine.arguments
let fallbackOnly = arguments.count == 4 && arguments[3] == "--fallback-only"
guard arguments.count == 3 || fallbackOnly else { fail("Expected input and output paths") }
let source = URL(fileURLWithPath: arguments[1])
let destination = URL(fileURLWithPath: arguments[2])
guard FileManager.default.fileExists(atPath: source.path) else { fail("File not found") }
// Own scratch files even when the helper is invoked directly for diagnostics.
let scratch = destination.deletingLastPathComponent().appendingPathComponent("fallback-" + UUID().uuidString)
do { try FileManager.default.createDirectory(at: scratch, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700]) }
catch { fail(error.localizedDescription) }
scratchToRemove = scratch
var browser: HTMLPreview?

func complete(_ image: CGImage) {
    do {
        try saveImage(image, to: destination)
        try? FileManager.default.removeItem(at: scratch)
        exit(0)
    } catch { fail(error.localizedDescription) }
}

func independentFallback(_ snapshot: URL, originalError: Error) {
    previewStage = "rendering the document without Quick Look"
    DispatchQueue.global(qos: .userInitiated).async {
        if let image = officeThumbnail(snapshot) { complete(image) }
        DispatchQueue.main.async {
            do {
                if let image = try wordPage(snapshot) { complete(image) }
                if ["html", "htm", "xhtml"].contains(source.pathExtension.lowercased()) {
                    // Use the original base URL so relative styles/images still load.
                    browser = HTMLPreview(source: source, destination: destination)
                    return
                }
                fail("Quick Look failed and this document has no usable saved preview: \(describe(originalError))")
            } catch { fail("Quick Look and document fallback failed: \(describe(error))") }
        }
    }
}

func fallback(_ originalError: Error) {
    previewStage = "waiting for macOS to provide readable file contents"
    DispatchQueue.global(qos: .userInitiated).async {
        do {
            let snapshot = try localSnapshot(source, in: scratch)
            DispatchQueue.main.async {
                if fallbackOnly { independentFallback(snapshot, originalError: originalError); return }
                previewStage = "rendering a temporary local copy"
                quickLook(snapshot) { result in
                    switch result {
                    case .success(let image): complete(image)
                    case .failure: independentFallback(snapshot, originalError: originalError)
                    }
                }
            }
        } catch {
            fail("Fallback could not obtain readable document contents: \(describe(error))")
        }
    }
}

if fallbackOnly {
    fallback(NSError(domain: "SwitchboardPreviewTest", code: 1,
        userInfo: [NSLocalizedDescriptionKey: "Quick Look bypassed for fallback verification"]))
} else {
    quickLook(source) { result in
        switch result {
        case .success(let image): complete(image)
        case .failure(let error): fallback(error)
        }
    }
}
RunLoop.main.run(until: Date(timeIntervalSinceNow: 20))
fail("Preview timed out after 20 seconds while \(previewStage)")

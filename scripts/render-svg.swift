import AppKit
import Foundation

let input = URL(fileURLWithPath: CommandLine.arguments[1])
let output = URL(fileURLWithPath: CommandLine.arguments[2])
let size = CGFloat(Double(CommandLine.arguments[3])!)
let data = try Data(contentsOf: input)

guard let image = NSImage(data: data) else {
    fputs("NSImage could not decode input\n", stderr)
    exit(2)
}

let pixels = Int(size.rounded())
guard let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: pixels,
    pixelsHigh: pixels,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bitmapFormat: [],
    bytesPerRow: 0,
    bitsPerPixel: 0
) else {
    fputs("Could not allocate bitmap\n", stderr)
    exit(3)
}

bitmap.size = NSSize(width: size, height: size)
NSGraphicsContext.saveGraphicsState()
guard let context = NSGraphicsContext(bitmapImageRep: bitmap) else {
    fputs("Could not create graphics context\n", stderr)
    exit(4)
}
NSGraphicsContext.current = context
NSColor.clear.setFill()
NSRect(x: 0, y: 0, width: size, height: size).fill()
image.draw(in: NSRect(x: 0, y: 0, width: size, height: size), from: .zero, operation: .sourceOver, fraction: 1)
context.flushGraphics()
NSGraphicsContext.restoreGraphicsState()

guard let png = bitmap.representation(using: .png, properties: [:]) else {
    fputs("Could not encode PNG\n", stderr)
    exit(5)
}

try png.write(to: output)

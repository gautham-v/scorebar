// The app icon's 1024pt master, drawn with AppKit rather than shipped as a
// vector asset: the mark is the same rounded scoreboard the menu bar draws, so
// keeping it in code keeps the two from drifting. Run via scripts/make-icon.sh,
// which renders this and folds the sizes into assets/AppIcon.icns.
//
//   swift scripts/make-icon.swift out.png

import AppKit

let S: CGFloat = 1024
let inset: CGFloat = 100
let bodyR: CGFloat = 185

let ink = NSColor(srgbRed: 0.12, green: 0.12, blue: 0.11, alpha: 1)

func rounded(_ r: NSRect, _ rad: CGFloat) -> NSBezierPath {
    NSBezierPath(roundedRect: r, xRadius: rad, yRadius: rad)
}

func render(_ draw: (NSRect) -> Void) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(S), pixelsHigh: Int(S),
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .calibratedRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let body = NSRect(x: inset, y: inset, width: S - 2*inset, height: S - 2*inset)
    let shape = rounded(body, bodyR)
    NSGraphicsContext.current!.cgContext.saveGState()
    shape.addClip()
    NSGradient(colors: [NSColor(srgbRed: 0.976, green: 0.972, blue: 0.960, alpha: 1),
                        NSColor(srgbRed: 0.898, green: 0.890, blue: 0.870, alpha: 1)])!
        .draw(in: body, angle: -90)
    NSGraphicsContext.current!.cgContext.restoreGState()
    // hairline edge so the icon keeps an edge on a white wallpaper
    NSColor(srgbRed: 0, green: 0, blue: 0, alpha: 0.10).setStroke()
    let edge = rounded(body.insetBy(dx: 1.5, dy: 1.5), bodyR - 1.5); edge.lineWidth = 3; edge.stroke()
    draw(body)
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

// The mark: a scoreboard, drawn as a rounded rectangle split down the middle —
// two sides, one number each, which is the whole of what scorebar shows. The
// frame is ink and the divider is the secondary grey the popover uses, so the
// icon carries the same one-colour, two-weights language as the app and reads
// at 16pt as a shape rather than as detail.
func scoreboard(_ body: NSRect) {
    let stroke: CGFloat = 62
    let board = NSRect(x: body.midX - 260, y: body.midY - 170, width: 520, height: 340)

    let frame = rounded(board, 72)
    frame.lineWidth = stroke
    ink.setStroke()
    frame.stroke()

    // Full height, so it meets the frame rather than floating inside it.
    let divider = NSBezierPath()
    divider.move(to: NSPoint(x: board.midX, y: board.minY))
    divider.line(to: NSPoint(x: board.midX, y: board.maxY))
    divider.lineWidth = 44
    ink.withAlphaComponent(0.30).setStroke()
    divider.stroke()
}

let out = CommandLine.arguments[1]
let rep = render(scoreboard)
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))

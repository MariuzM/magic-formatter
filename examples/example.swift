import Foundation
import SwiftUI

struct Config {
    var name: String
    var retries: Int
}

func create(_ a: Int, _ b: Int, _ c: Int) -> Int { a + b + c }

func magicTrailingComma() -> Int {
    create(
        1,
        2,
        3,
    )
}

func firstArgumentOnNextLineExpands() -> Int {
    create(
        1,
        2,
        3
    )
}

func firstArgumentOnSameLineCollapses() -> Int {
    create(1, 2, 3)
}

func singleLineIf(value: Int) -> Int {
    if value == 0 { return 1 }
    if value > 10 {
        return 10
    }
    return value
}

func methodChainExpands(items: [Int]) -> [Int] {
    items
        .filter { $0 > 1 }
        .map { $0 * 2 }
        .sorted()
}

func methodChainStaysInline(items: [Int]) -> [Int] {
    items.filter { $0 > 1 }.map { $0 * 2 }
}

func alignedAssignments(config: inout Config) -> Int {
    config.name    = "example"
    config.retries = 3
    let total      = config.retries + 1

    let unalignedAfterBlankLine = total * 2
    return unalignedAfterBlankLine
}

func messySpacing(a: Int, b: Int) -> Int {
    let sum = a + b
    return sum * 2
}

func reindentsControlFlow(value: Int?) -> String {
    guard let value = value else {
        return "none"
    }
    switch value {
    case 0:
        return "zero"
    case 1, 2:
        return "small"
    default:
        return "large"
    }
}

func multiLineCondition(a: Int?, b: Int) {
    if let a = a,
        a > b {
        print(a)
    }
}

func multiLineString() -> String {
    let text = """
        first line
          indented line
        """
    return text
}

func conditionalCompilation() -> String {
    #if DEBUG
        return "debug"
    #else
        return "release"
    #endif
}

func grammarGapIsLeftUntouched(payload: [String: Any]) -> String {
    let message = payload["message"] as? String ?? "fallback"
    return message
}

struct ContentView: View {
    var body: some View {
        VStack(spacing: 16) {
            Text("Title")
                .font(.title)
            Text("Subtitle").font(.body).foregroundStyle(.secondary)
        }
        .padding(24)
        .frame(width: 320)
    }
}

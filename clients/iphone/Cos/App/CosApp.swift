import SwiftUI

@main
struct CosApp: App {
    @State private var app = AppModel.live()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(app)
        }
    }
}

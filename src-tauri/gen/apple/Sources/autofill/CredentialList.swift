import SwiftUI

/// What the sheet is showing. The controller moves it along; the view only
/// draws it and reports taps.
enum SheetState {
    case unlocking
    case locked(String)
    case credentials([Credential])
}

final class SheetModel: ObservableObject {
    @Published var state = SheetState.unlocking
    @Published var query = ""
}

/// The AutoFill sheet: the logins for the site by title and user name — never
/// a password, which is read only for the row that is tapped.
struct CredentialList: View {
    @ObservedObject var model: SheetModel
    let retry: () -> Void
    let select: (Credential) -> Void
    let cancel: () -> Void

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Rowel")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel", action: cancel)
                    }
                }
        }
    }

    @ViewBuilder private var content: some View {
        switch model.state {
        case .unlocking:
            ProgressView()
        case let .locked(message):
            VStack(spacing: 16) {
                Image(systemName: "lock.fill")
                    .font(.largeTitle)
                    .foregroundStyle(.secondary)
                Text(message)
                    .multilineTextAlignment(.center)
                Button("Try Again", action: retry)
                    .buttonStyle(.borderedProminent)
            }
            .padding()
        case let .credentials(credentials):
            let shown = matching(credentials)
            List(shown, id: \.record) { credential in
                Button { select(credential) } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(credential.title)
                            .foregroundStyle(.primary)
                        Text(credential.user.isEmpty ? credential.host : credential.user)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .overlay {
                if shown.isEmpty {
                    Text("No matching logins.")
                        .foregroundStyle(.secondary)
                }
            }
            .searchable(text: $model.query)
        }
    }

    private func matching(_ credentials: [Credential]) -> [Credential] {
        let query = model.query
        guard !query.isEmpty else { return credentials }
        return credentials.filter {
            $0.title.localizedCaseInsensitiveContains(query)
                || $0.user.localizedCaseInsensitiveContains(query)
                || $0.host.localizedCaseInsensitiveContains(query)
        }
    }
}

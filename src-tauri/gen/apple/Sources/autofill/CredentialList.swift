import SwiftUI

/// What the sheet is showing. The controller moves it along; the view only
/// draws it and reports taps.
enum SheetState {
    case unlocking
    /// The vault did not open; trying again asks for Face ID again.
    case locked(String)
    /// A request that cannot be answered, said once: there is nothing to retry.
    case failed(String)
    /// The site's passkeys, when iOS asked for a passkey sign-in, then its
    /// logins.
    case credentials(passkeys: [PasskeyAccount], passwords: [Credential])
    /// A passkey registration waiting on the user's yes.
    case registration(relyingParty: String, userName: String)
    /// A row was picked and the vault is answering for it: nothing to tap,
    /// and no Cancel, until it has — a second tap would answer twice, and a
    /// cancel would not stop what is already under way.
    case answering

    var isAnswering: Bool {
        if case .answering = self { return true }
        return false
    }
}

final class SheetModel: ObservableObject {
    @Published var state = SheetState.unlocking
    @Published var query = ""
}

/// The AutoFill sheet: the passkeys and logins for the site by name — never a
/// password, which is read only for the row that is tapped — or a passkey
/// registration's confirmation.
struct CredentialList: View {
    @ObservedObject var model: SheetModel
    let retry: () -> Void
    let select: (Credential) -> Void
    let selectPasskey: (PasskeyAccount) -> Void
    let register: () -> Void
    let cancel: () -> Void

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Rowel")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel", action: cancel)
                            .disabled(model.state.isAnswering)
                    }
                }
        }
    }

    @ViewBuilder private var content: some View {
        switch model.state {
        case .unlocking, .answering:
            ProgressView()
        case let .locked(message):
            Notice(systemImage: "lock.fill", message: message) {
                Button("Try Again", action: retry)
                    .buttonStyle(.borderedProminent)
            }
        case let .failed(message):
            Notice(systemImage: "exclamationmark.triangle.fill", message: message) {}
        case let .credentials(passkeys, passwords):
            let shownPasskeys = matching(passkeys)
            let shownPasswords = matching(passwords)
            // One list: the passkeys first, since a passkey sign-in is what
            // iOS asked for when there are any, and the logins below them.
            List {
                if !shownPasskeys.isEmpty {
                    Section("Passkeys") {
                        // The whole record names a row: imports can put one
                        // credential id on two logins.
                        ForEach(shownPasskeys, id: \.self) { account in
                            Button { selectPasskey(account) } label: {
                                Row(title: account.userName, detail: account.userDisplayName)
                            }
                        }
                    }
                }
                Section {
                    ForEach(shownPasswords, id: \.record) { credential in
                        Button { select(credential) } label: {
                            Row(title: credential.title,
                                detail: credential.user.isEmpty ? credential.host : credential.user)
                        }
                    }
                } header: {
                    if !shownPasskeys.isEmpty && !shownPasswords.isEmpty {
                        Text("Passwords")
                    }
                }
            }
            .overlay {
                if shownPasskeys.isEmpty && shownPasswords.isEmpty {
                    Text("No matching logins.")
                        .foregroundStyle(.secondary)
                }
            }
            .searchable(text: $model.query)
        case let .registration(relyingParty, userName):
            // The registration's consent. Face ID opened the vault; this is
            // the yes to the passkey itself, named as the site asked for it.
            Notice(systemImage: "person.badge.key.fill", message: "Save a passkey for \(relyingParty)?") {
                Text(userName)
                    .foregroundStyle(.secondary)
                Button("Save Passkey", action: register)
                    .buttonStyle(.borderedProminent)
                Button("Not Now", action: cancel)
            }
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

    private func matching(_ passkeys: [PasskeyAccount]) -> [PasskeyAccount] {
        let query = model.query
        guard !query.isEmpty else { return passkeys }
        return passkeys.filter {
            $0.userName.localizedCaseInsensitiveContains(query)
                || $0.userDisplayName.localizedCaseInsensitiveContains(query)
        }
    }
}

private struct Row: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .foregroundStyle(.primary)
            if !detail.isEmpty {
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// A message in the middle of the sheet, with what can be done about it.
private struct Notice<Actions: View>: View {
    let systemImage: String
    let message: String
    @ViewBuilder let actions: () -> Actions

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: systemImage)
                .font(.largeTitle)
                .foregroundStyle(.secondary)
            Text(message)
                .multilineTextAlignment(.center)
            actions()
        }
        .padding()
    }
}

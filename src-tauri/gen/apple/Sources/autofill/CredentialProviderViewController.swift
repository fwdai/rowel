import AuthenticationServices
import SwiftUI

/// The AutoFill extension's entry point (`NSExtensionPrincipalClass`). iOS
/// asks it for one of two things: the list of logins for a site, or the
/// credential behind one identity the app published and the user tapped in
/// QuickType. Either way the vault opens only after Face ID, so every answer
/// goes through the sheet. Passwords only; passkeys are not served yet.
final class CredentialProviderViewController: ASCredentialProviderViewController {
    private let model = SheetModel()
    private var vault: Vault?
    /// The sites iOS is filling for, in list mode.
    private var serviceIdentifiers: [String] = []
    /// The one identity iOS asked for, in QuickType mode.
    private var record: String?

    override func viewDidLoad() {
        super.viewDidLoad()
        let sheet = UIHostingController(rootView: CredentialList(
            model: model,
            retry: { [weak self] in self?.unlock() },
            select: { [weak self] in self?.fill($0.record) },
            cancel: { [weak self] in self?.cancel(.userCanceled) }
        ))
        addChild(sheet)
        sheet.view.frame = view.bounds
        sheet.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(sheet.view)
        sheet.didMove(toParent: self)
    }

    // MARK: - The list

    override func prepareCredentialList(for serviceIdentifiers: [ASCredentialServiceIdentifier]) {
        self.serviceIdentifiers = serviceIdentifiers.map(\.identifier)
        unlock()
    }

    // MARK: - One identity from QuickType

    // The key is behind Face ID, and there is no Face ID without a sheet: iOS
    // is told to come back through `prepareInterfaceToProvideCredential`.
    override func provideCredentialWithoutUserInteraction(for credentialIdentity: ASPasswordCredentialIdentity) {
        cancel(.userInteractionRequired)
    }

    @available(iOS 17.0, *)
    override func provideCredentialWithoutUserInteraction(for credentialRequest: ASCredentialRequest) {
        cancel(.userInteractionRequired)
    }

    override func prepareInterfaceToProvideCredential(for credentialIdentity: ASPasswordCredentialIdentity) {
        provide(credentialIdentity.recordIdentifier)
    }

    @available(iOS 17.0, *)
    override func prepareInterfaceToProvideCredential(for credentialRequest: ASCredentialRequest) {
        // Only passwords are declared (`ProvidesPasswords`), so a passkey
        // request is not expected here until passkeys are served.
        guard let request = credentialRequest as? ASPasswordCredentialRequest else {
            return cancel(.failed)
        }
        provide(request.credentialIdentity.recordIdentifier)
    }

    private func provide(_ record: String?) {
        guard let record else { return cancel(.credentialIdentityNotFound) }
        self.record = record
        unlock()
    }

    // MARK: - Unlocking and answering

    private func unlock() {
        model.state = .unlocking
        Task { @MainActor in
            do {
                let vault = try await Unlock.vault()
                self.vault = vault
                if let record {
                    fill(record)
                } else {
                    model.state = try .credentials(vault.credentialsFor(serviceIdentifiers: serviceIdentifiers))
                }
            } catch {
                model.state = .locked(Unlock.message(for: error))
            }
        }
    }

    // Unseals the one row chosen, and hands iOS its name and password.
    private func fill(_ record: String) {
        guard let vault else { return }
        do {
            let password = try vault.password(record: record)
            extensionContext.completeRequest(
                withSelectedCredential: ASPasswordCredential(user: password.user, password: password.password)
            )
        } catch AutofillError.NotFound {
            // Deleted, or of a workspace that is not the open one, since the
            // app last published its identities.
            cancel(.credentialIdentityNotFound)
        } catch {
            model.state = .locked(Unlock.message(for: error))
        }
    }

    private func cancel(_ code: ASExtensionError.Code) {
        extensionContext.cancelRequest(withError: ASExtensionError(code))
    }
}

import AuthenticationServices
import SwiftUI

/// The AutoFill extension's entry point (`NSExtensionPrincipalClass`). iOS
/// asks it for one of two things: the list of logins for a site, or the
/// credential behind one identity the app published and the user tapped in
/// QuickType. Either way the vault opens only after Face ID, so every answer
/// goes through the sheet. Passwords only; passkeys are not served yet.
///
/// What iOS hands over is never trusted for what to fill: every fill names
/// the sites being filled for, and the vault decides whether the login is
/// still one of theirs. The vault's file and database work runs off the main
/// actor, so the sheet — its Cancel above all — stays live through it.
final class CredentialProviderViewController: ASCredentialProviderViewController {
    private let model = SheetModel()
    private var vault: Vault?
    /// The sites iOS is filling for: the list's, or the one the tapped
    /// QuickType suggestion was published under.
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
        provide(credentialIdentity.recordIdentifier, site: credentialIdentity.serviceIdentifier.identifier)
    }

    @available(iOS 17.0, *)
    override func prepareInterfaceToProvideCredential(for credentialRequest: ASCredentialRequest) {
        // Only passwords are declared (`ProvidesPasswords`), so a passkey
        // request is not expected here until passkeys are served.
        guard let request = credentialRequest as? ASPasswordCredentialRequest else {
            return cancel(.failed)
        }
        let identity = request.credentialIdentity
        provide(identity.recordIdentifier, site: identity.serviceIdentifier.identifier)
    }

    // The identity names the record and the site iOS offered it for. Both go
    // to the vault: the app published the identity some time ago, and the
    // login may since have been moved to another site, or deleted.
    private func provide(_ record: String?, site: String) {
        guard let record else { return cancel(.credentialIdentityNotFound) }
        self.record = record
        serviceIdentifiers = [site]
        unlock()
    }

    // MARK: - Unlocking and answering

    private func unlock() {
        model.state = .unlocking
        let sites = serviceIdentifiers
        let record = record
        Task { [weak self] in
            do {
                let (vault, credentials) = try await Task.detached(priority: .userInitiated) {
                    let vault = try await Unlock.vault()
                    let credentials: [Credential] = try record == nil
                        ? vault.credentialsFor(serviceIdentifiers: sites)
                        : []
                    return (vault, credentials)
                }.value
                guard let self else { return }
                self.vault = vault
                if let record {
                    fill(record)
                } else {
                    model.state = .credentials(credentials)
                }
            } catch {
                self?.model.state = .locked(Unlock.message(for: error))
            }
        }
    }

    // Unseals the one row chosen, for the sites being filled, and hands iOS
    // its name and password. One answer at a time: the sheet is busy from
    // here until iOS has it, and a tap that was already queued finds it so.
    private func fill(_ record: String) {
        guard let vault, !model.state.isAnswering else { return }
        model.state = .answering
        let sites = serviceIdentifiers
        Task { [weak self] in
            do {
                let password = try await Task.detached(priority: .userInitiated) {
                    try vault.password(record: record, serviceIdentifiers: sites)
                }.value
                self?.extensionContext.completeRequest(
                    withSelectedCredential: ASPasswordCredential(user: password.user, password: password.password)
                )
            } catch AutofillError.NotFound {
                // Gone, moved to another site, or of a workspace that is not
                // the open one, since the app last published its identities.
                self?.cancel(.credentialIdentityNotFound)
            } catch {
                self?.model.state = .locked(Unlock.message(for: error))
            }
        }
    }

    private func cancel(_ code: ASExtensionError.Code) {
        extensionContext.cancelRequest(withError: ASExtensionError(code))
    }
}

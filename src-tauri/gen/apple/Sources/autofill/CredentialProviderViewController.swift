import AuthenticationServices
import SwiftUI

/// The AutoFill extension's entry point (`NSExtensionPrincipalClass`). iOS
/// asks it for the list of logins for a site, for the credential behind one
/// identity the app published and the user tapped in QuickType, or — from iOS
/// 17 — for a passkey sign-in or registration. Every answer opens the vault,
/// and the vault opens only after Face ID, so every answer goes through the
/// sheet.
///
/// iOS is the WebAuthn client: it hands over the hash of the client data it
/// wrote, which is what gets signed. The Face ID unlock is the user
/// verification; the row tapped, or the registration's confirmation, is the
/// user's yes, and Rust is told it (`rowel-autofill`).
///
/// What iOS hands over is never trusted for what to fill: every fill names
/// the sites being filled for, and the vault decides whether the login is
/// still one of theirs. The vault's file and database work runs off the main
/// actor, so the sheet — its Cancel above all — stays live through it.
final class CredentialProviderViewController: ASCredentialProviderViewController {
    /// What iOS asked for, read off whichever entry point it called.
    private enum Request {
        /// The sheet's list for the sites iOS names, with the passkey sign-in
        /// iOS asked for alongside, when it asked for one.
        case list(serviceIdentifiers: [String], passkey: PasskeyAssertion?)
        /// One password identity from QuickType, and the site iOS offered
        /// it for.
        case password(record: String, site: String)
        /// One passkey identity from QuickType.
        case passkey(PasskeyAssertion)
        /// A passkey registration, made once the user allows it.
        case registration(PasskeyRegistration)

        var reason: String {
            switch self {
            case .list(_, .none), .password: "Unlock Rowel to fill a password."
            case .list, .passkey: "Unlock Rowel to sign in with a passkey."
            case .registration: "Unlock Rowel to save a passkey."
            }
        }
    }

    private let model = SheetModel()
    private var vault: Vault?
    private var request = Request.list(serviceIdentifiers: [], passkey: nil)
    /// The sites a fill is for: the list's, or the one the tapped QuickType
    /// suggestion was published under.
    private var sites: [String] {
        switch request {
        case let .list(serviceIdentifiers, _): serviceIdentifiers
        case let .password(_, site): [site]
        default: []
        }
    }
    /// What Cancel tells iOS: the user's choice, unless the request already
    /// failed for a reason of its own.
    private var failure = ASExtensionError.Code.userCanceled

    override func viewDidLoad() {
        super.viewDidLoad()
        let sheet = UIHostingController(rootView: CredentialList(
            model: model,
            retry: { [weak self] in self?.unlock() },
            select: { [weak self] in self?.fill($0.record) },
            selectPasskey: { [weak self] in self?.signIn(as: $0) },
            register: { [weak self] in self?.register() },
            cancel: { [weak self] in
                guard let self else { return }
                cancel(failure)
            }
        ))
        addChild(sheet)
        sheet.view.frame = view.bounds
        sheet.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(sheet.view)
        sheet.didMove(toParent: self)
    }

    // MARK: - The list

    override func prepareCredentialList(for serviceIdentifiers: [ASCredentialServiceIdentifier]) {
        request = .list(serviceIdentifiers: serviceIdentifiers.map(\.identifier), passkey: nil)
        unlock()
    }

    // A passkey sign-in: the site's passkeys the relying party allows, and
    // its logins below them, since either answers the request.
    @available(iOS 17.0, *)
    override func prepareCredentialList(
        for serviceIdentifiers: [ASCredentialServiceIdentifier],
        requestParameters: ASPasskeyCredentialRequestParameters
    ) {
        request = .list(
            serviceIdentifiers: serviceIdentifiers.map(\.identifier),
            passkey: PasskeyAssertion(
                rpId: requestParameters.relyingPartyIdentifier,
                clientDataHash: requestParameters.clientDataHash,
                allowedCredentialIds: requestParameters.allowedCredentials,
                record: nil
            )
        )
        unlock()
    }

    // MARK: - One identity from QuickType

    // The key is behind Face ID, and there is no Face ID without a sheet: iOS
    // is told to come back through `prepareInterfaceToProvideCredential`.
    override func provideCredentialWithoutUserInteraction(for credentialIdentity: ASPasswordCredentialIdentity) {
        cancel(.userInteractionRequired)
    }

    // A password or a passkey alike: signing needs the key as much as filling.
    @available(iOS 17.0, *)
    override func provideCredentialWithoutUserInteraction(for credentialRequest: ASCredentialRequest) {
        cancel(.userInteractionRequired)
    }

    // The identity names the record and the site iOS offered it for. Both go
    // to the vault: the app published the identity some time ago, and the
    // login may since have been moved to another site, or deleted.
    override func prepareInterfaceToProvideCredential(for credentialIdentity: ASPasswordCredentialIdentity) {
        provide(credentialIdentity.recordIdentifier.map {
            .password(record: $0, site: credentialIdentity.serviceIdentifier.identifier)
        })
    }

    @available(iOS 17.0, *)
    override func prepareInterfaceToProvideCredential(for credentialRequest: ASCredentialRequest) {
        switch credentialRequest {
        case let request as ASPasswordCredentialRequest:
            let identity = request.credentialIdentity
            provide(identity.recordIdentifier.map {
                .password(record: $0, site: identity.serviceIdentifier.identifier)
            })
        case let request as ASPasskeyCredentialRequest:
            // The identity names the passkey itself: its login by record, and
            // which of the login's passkeys by credential id.
            let identity = request.credentialIdentity as? ASPasskeyCredentialIdentity
            provide(identity.flatMap { identity in
                identity.recordIdentifier.map {
                    .passkey(PasskeyAssertion(
                        rpId: identity.relyingPartyIdentifier,
                        clientDataHash: request.clientDataHash,
                        allowedCredentialIds: [identity.credentialID],
                        record: $0
                    ))
                }
            })
        default:
            cancel(.failed)
        }
    }

    private func provide(_ request: Request?) {
        guard let request else { return cancel(.credentialIdentityNotFound) }
        self.request = request
        unlock()
    }

    // MARK: - A passkey registration

    @available(iOS 17.0, *)
    override func prepareInterface(forPasskeyRegistration registrationRequest: ASCredentialRequest) {
        guard let request = registrationRequest as? ASPasskeyCredentialRequest,
              let identity = request.credentialIdentity as? ASPasskeyCredentialIdentity
        else { return cancel(.failed) }
        // iOS 17 does not say which credentials the site already has; from
        // 18 it does, and the vault refuses to make a second for the account.
        var excluded: [Data] = []
        if #available(iOS 18.0, *) {
            excluded = request.excludedCredentials?.map(\.credentialID) ?? []
        }
        self.request = .registration(PasskeyRegistration(
            rpId: identity.relyingPartyIdentifier,
            // Not in iOS's request: the new login is named after the rpId.
            rpName: nil,
            userName: identity.userName,
            userDisplayName: nil,
            userHandle: identity.userHandle,
            clientDataHash: request.clientDataHash,
            excludedCredentialIds: excluded,
            supportedAlgorithms: request.supportedAlgorithms.map { Int64($0.rawValue) }
        ))
        unlock()
    }

    // MARK: - Unlocking and answering

    private func unlock() {
        model.state = .unlocking
        let request = request
        let reason = request.reason
        var listing: (serviceIdentifiers: [String], passkey: PasskeyAssertion?)?
        if case let .list(serviceIdentifiers, passkey) = request {
            listing = (serviceIdentifiers, passkey)
        }
        Task { [weak self] in
            do {
                // The keychain read, the open and the listing are file and
                // database work: off the main actor.
                let (vault, passkeys, passwords) = try await Task.detached(priority: .userInitiated) {
                    let vault = try await Unlock.vault(reason: reason)
                    guard let listing else { return (vault, [PasskeyAccount](), [Credential]()) }
                    let passkeys = try listing.passkey.map {
                        try vault.passkeysFor(rpId: $0.rpId, allowedCredentialIds: $0.allowedCredentialIds)
                    } ?? []
                    let passwords = try vault.credentialsFor(serviceIdentifiers: listing.serviceIdentifiers)
                    return (vault, passkeys, passwords)
                }.value
                guard let self else { return }
                self.vault = vault
                switch request {
                case .list:
                    model.state = .credentials(passkeys: passkeys, passwords: passwords)
                case let .password(record, _):
                    fill(record)
                case let .passkey(assertion):
                    signIn(assertion)
                case let .registration(registration):
                    model.state = .registration(relyingParty: registration.rpId, userName: registration.userName)
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
        let sites = sites
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

    // A passkey row tapped: the sign-in iOS asked for, as that account — its
    // login by record, and the passkey by narrowing the allow list to it, as
    // a login can carry several.
    private func signIn(as account: PasskeyAccount) {
        guard case var .list(_, passkey?) = request else { return }
        passkey.allowedCredentialIds = [account.credentialId]
        passkey.record = account.record
        signIn(passkey)
    }

    private func signIn(_ assertion: PasskeyAssertion) {
        guard #available(iOS 17.0, *), let vault, !model.state.isAnswering else { return }
        model.state = .answering
        Task { [weak self] in
            do {
                let signed = try await Task.detached(priority: .userInitiated) {
                    try vault.assertPasskey(request: assertion)
                }.value
                _ = await self?.extensionContext.completeAssertionRequest(using: ASPasskeyAssertionCredential(
                    userHandle: signed.userHandle,
                    relyingParty: assertion.rpId,
                    signature: signed.signature,
                    clientDataHash: assertion.clientDataHash,
                    authenticatorData: signed.authenticatorData,
                    credentialID: signed.credentialId
                ))
            } catch {
                self?.fail(error)
            }
        }
    }

    // "Save Passkey": makes it, and hands iOS what the relying party gets.
    // QuickType learns of it when the app next runs and publishes its
    // identities; until then the list offers it. Busy from the tap until iOS
    // has the answer: a second tap cannot make a second passkey, and a Cancel
    // cannot land after the vault has kept one.
    private func register() {
        guard #available(iOS 17.0, *), let vault, !model.state.isAnswering,
              case let .registration(registration) = request else { return }
        model.state = .answering
        Task { [weak self] in
            do {
                let made = try await Task.detached(priority: .userInitiated) {
                    try vault.registerPasskey(request: registration)
                }.value
                _ = await self?.extensionContext.completeRegistrationRequest(using: ASPasskeyRegistrationCredential(
                    relyingParty: registration.rpId,
                    clientDataHash: registration.clientDataHash,
                    credentialID: made.credentialId,
                    attestationObject: made.attestationObject
                ))
            } catch {
                self?.fail(error)
            }
        }
    }

    // A passkey ceremony that did not happen. Gone or not picked ends the
    // request at once; anything else is said on the sheet, and Cancel tells
    // iOS why.
    private func fail(_ error: Error) {
        switch error {
        case AutofillError.NotFound:
            return cancel(.credentialIdentityNotFound)
        case AutofillError.Denied:
            return cancel(.userCanceled)
        case AutofillError.Excluded:
            if #available(iOS 18.0, *) {
                failure = .matchedExcludedCredential
            } else {
                failure = .failed
            }
        default:
            failure = .failed
        }
        model.state = .failed(Unlock.message(for: error))
    }

    private func cancel(_ code: ASExtensionError.Code) {
        extensionContext.cancelRequest(withError: ASExtensionError(code))
    }
}

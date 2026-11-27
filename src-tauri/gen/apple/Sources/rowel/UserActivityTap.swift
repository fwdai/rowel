import Foundation
import ObjectiveC

/// Hands every `NSUserActivity` the app is launched or continued with to a
/// handler, then lets the delegate that owns the callback carry on. The
/// handler answers whether the activity was its to take.
///
/// The app and scene delegates are tao's, written in Rust, and forward an
/// activity only when it carries a web URL (a universal link). Anything else —
/// a credential exchange among them — is dropped there, so the three callbacks
/// an activity can arrive through are wrapped in place:
///
/// - `application:continueUserActivity:restorationHandler:`, the app delegate
///   lifecycle, launched or already running;
/// - `scene:continueUserActivity:`, the scene lifecycle, already running;
/// - `scene:willConnectToSession:options:`, the scene lifecycle, launched: the
///   activities are in the connection options and nowhere else.
enum UserActivityTap {
    typealias Handler = (_ activity: NSUserActivity, _ via: String) -> Bool

    /// Which of the three callbacks were wrapped (`replaced`: the delegate
    /// had one; `added`: it had none), by selector name.
    private(set) static var installed: [String: String] = [:]

    static func install(appDelegate: AnyClass?, sceneDelegate: AnyClass?, handler: @escaping Handler) {
        if let appDelegate {
            wrapApplicationContinue(on: appDelegate, handler)
        }
        if let sceneDelegate {
            wrapSceneContinue(on: sceneDelegate, handler)
            wrapSceneWillConnect(on: sceneDelegate, handler)
        }
    }

    // The restoration handler and the session are passed through untouched,
    // as raw pointers, so nothing here retains a block it did not copy.

    private static func wrapApplicationContinue(on cls: AnyClass, _ handler: @escaping Handler) {
        typealias Original = @convention(c) (
            AnyObject, Selector, UnsafeRawPointer?, NSUserActivity, UnsafeRawPointer?
        ) -> Bool
        let name = "application:continueUserActivity:restorationHandler:"
        let selector = NSSelectorFromString(name)
        var original: Original?
        let block: @convention(block) (
            AnyObject, UnsafeRawPointer?, NSUserActivity, UnsafeRawPointer?
        ) -> Bool = { receiver, application, activity, restoration in
            // UIKit is told the activity was handled when either took it:
            // tao answers no to everything but a universal link.
            let taken = handler(activity, name)
            let forwarded = original?(receiver, selector, application, activity, restoration) ?? false
            return taken || forwarded
        }
        original = wrap(cls, selector, block, types: "B@:@@@").map {
            unsafeBitCast($0, to: Original.self)
        }
        installed[name] = original == nil ? "added" : "replaced"
    }

    private static func wrapSceneContinue(on cls: AnyClass, _ handler: @escaping Handler) {
        typealias Original = @convention(c) (
            AnyObject, Selector, UnsafeRawPointer?, NSUserActivity
        ) -> Void
        let name = "scene:continueUserActivity:"
        let selector = NSSelectorFromString(name)
        var original: Original?
        let block: @convention(block) (AnyObject, UnsafeRawPointer?, NSUserActivity) -> Void = {
            receiver, scene, activity in
            _ = handler(activity, name)
            original?(receiver, selector, scene, activity)
        }
        original = wrap(cls, selector, block, types: "v@:@@").map {
            unsafeBitCast($0, to: Original.self)
        }
        installed[name] = original == nil ? "added" : "replaced"
    }

    private static func wrapSceneWillConnect(on cls: AnyClass, _ handler: @escaping Handler) {
        typealias Original = @convention(c) (
            AnyObject, Selector, UnsafeRawPointer?, UnsafeRawPointer?, NSObject
        ) -> Void
        let name = "scene:willConnectToSession:options:"
        let selector = NSSelectorFromString(name)
        var original: Original?
        let block: @convention(block) (
            AnyObject, UnsafeRawPointer?, UnsafeRawPointer?, NSObject
        ) -> Void = { receiver, scene, session, options in
            // The delegate first: it is what builds the window the handler
            // may want to present on.
            original?(receiver, selector, scene, session, options)
            // `UIScene.ConnectionOptions.userActivities`, read by name so this
            // file needs no UIKit.
            let activities = options.value(forKey: "userActivities") as? Set<NSUserActivity> ?? []
            for activity in activities {
                _ = handler(activity, name)
            }
        }
        original = wrap(cls, selector, block, types: "v@:@@@").map {
            unsafeBitCast($0, to: Original.self)
        }
        installed[name] = original == nil ? "added" : "replaced"
    }

    /// Put `block` in place of `selector` on `cls` and return what was there,
    /// or add it when the class has no such method of its own.
    private static func wrap(_ cls: AnyClass, _ selector: Selector, _ block: Any, types: String) -> IMP? {
        let replacement = imp_implementationWithBlock(block)
        // `class_getInstanceMethod` also finds a superclass's method, and
        // setting that one's implementation would change every subclass.
        guard let method = ownMethod(of: cls, selector) else {
            class_addMethod(cls, selector, replacement, types)
            return nil
        }
        return method_setImplementation(method, replacement)
    }

    private static func ownMethod(of cls: AnyClass, _ selector: Selector) -> Method? {
        var count: UInt32 = 0
        guard let methods = class_copyMethodList(cls, &count) else { return nil }
        defer { free(methods) }
        return (0..<Int(count)).map { methods[$0] }.first { method_getName($0) == selector }
    }
}

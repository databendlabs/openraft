use crate::RaftTypeConfig;
use crate::core::notification::Notification;

/// What ended one call of [`Notifications::deliver_local`].
///
/// [`Notifications::deliver_local`]: crate::core::notifications::Notifications::deliver_local
pub(crate) enum LocalDelivery<C>
where C: RaftTypeConfig
{
    /// Every buffered self-notification is in the notification channel.
    Delivered,

    /// A notification was received while a self-notification waited for channel capacity.
    ///
    /// The caller handles it and calls again; the pending send is kept.
    Received(Notification<C>),

    /// Shutdown was requested.
    Shutdown,

    /// The notification channel is closed.
    Closed,
}

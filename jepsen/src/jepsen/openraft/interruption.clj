(ns jepsen.openraft.interruption)

(defn interruption?
  "True when e signals that the current thread was interrupted, in any of the
  forms the HTTP client and jepsen.control.sshj produce."
  [e]
  (or (instance? InterruptedException e)
      ;; SocketTimeoutException extends InterruptedIOException but signals an
      ;; expired socket read deadline, not a thread interruption.
      (and (instance? java.io.InterruptedIOException e)
           (not (instance? java.net.SocketTimeoutException e)))
      (instance? java.nio.channels.ClosedByInterruptException e)
      (= :interrupted (:kind (ex-data e)))))

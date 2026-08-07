(ns jepsen.openraft.interruption)

(defn interruption?
  "True when an exception signals that the current thread was interrupted."
  [e]
  (or (instance? InterruptedException e)
      (instance? java.io.InterruptedIOException e)
      (instance? java.nio.channels.ClosedByInterruptException e)
      (= :interrupted (:kind (ex-data e)))))

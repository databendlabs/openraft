(ns jepsen.openraft.interruption-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.interruption :as interruption]))

(deftest recognizes-every-interruption-form
  (doseq [[label error]
          [[:interrupted-exception
            (InterruptedException. "interrupted")]
           [:interrupted-io
            (java.io.InterruptedIOException. "interrupted")]
           [:closed-by-interrupt
            (java.nio.channels.ClosedByInterruptException.)]
           [:wrapped
            (ex-info "interrupted" {:kind :interrupted})]]]
    (testing (name label)
      (is (interruption/interruption? error)))))

(deftest rejects-transport-failures
  (doseq [[label error]
          [[:socket-timeout
            (java.net.SocketTimeoutException. "read timed out")]
           [:transport-error
            (ex-info "HTTP request failed" {:kind :transport-error})]
           [:io
            (java.io.IOException. "broken pipe")]]]
    (testing (name label)
      (is (not (interruption/interruption? error))))))

(deftest recognizes-fatal-throwables
  (is (interruption/fatal-throwable? (ThreadDeath.)))
  (is (interruption/fatal-throwable? (StackOverflowError.)))
  (is (not (interruption/fatal-throwable? (RuntimeException.)))))

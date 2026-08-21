(ns jepsen.openraft.harness-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.harness :as harness]))

(deftest starts-without-a-primary-failure
  (is (nil? (harness/primary-failure (harness/failure-state)))))

(deftest records-the-first-failure
  (let [state (harness/failure-state)
        context {:operation :write
                 :node :n1}
        throwable (ex-info "Client failed" {:kind :client-error})]
    (harness/record-failure! state :client context throwable)
    (let [failure (harness/primary-failure state)]
      (is (= :client (:source failure)))
      (is (= context (:context failure)))
      (is (identical? throwable (:throwable failure))))))

(deftest retains-the-first-failure
  (let [state (harness/failure-state)
        first-throwable (RuntimeException. "first")
        later-throwable (RuntimeException. "later")]
    (harness/record-failure! state
                             :nemesis
                             {:operation :partition}
                             first-throwable)
    (harness/record-failure! state
                             :teardown
                             {:operation :heal}
                             later-throwable)
    (let [failure (harness/primary-failure state)]
      (is (= :nemesis (:source failure)))
      (is (= {:operation :partition} (:context failure)))
      (is (identical? first-throwable (:throwable failure))))))

(deftest ignores-interruptions
  (doseq [[description throwable]
          [["InterruptedException" (InterruptedException. "interrupted")]
           ["InterruptedIOException"
            (java.io.InterruptedIOException. "interrupted")]
           ["ClosedByInterruptException"
            (java.nio.channels.ClosedByInterruptException.)]
           ["interrupted ex-info"
            (ex-info "interrupted" {:kind :interrupted})]]]
    (testing description
      (let [state (harness/failure-state)]
        (harness/record-failure! state
                                 :client
                                 {:operation :read}
                                 throwable)
        (is (nil? (harness/primary-failure state)))))))

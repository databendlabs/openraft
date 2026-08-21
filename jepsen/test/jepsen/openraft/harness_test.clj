(ns jepsen.openraft.harness-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.harness :as harness]))

(deftest starts-without-a-primary-failure
  (let [state (harness/failure-state)]
    (is (nil? (harness/primary-failure state)))
    (is (empty? (harness/secondary-failures state)))))

(deftest records-the-first-failure
  (let [state (harness/failure-state)
        context {:operation :write
                 :node :n1}
        throwable (ex-info "Client failed" {:kind :client-error})]
    (is (true? (harness/record-failure!
                state
                :client
                context
                throwable)))
    (let [failure (harness/primary-failure state)]
      (is (= :client (:source failure)))
      (is (= context (:context failure)))
      (is (identical? throwable (:throwable failure))))))

(deftest retains-the-first-failure
  (let [state (harness/failure-state)
        first-throwable (RuntimeException. "first")
        later-throwable (RuntimeException. "later")
        last-throwable (RuntimeException. "last")]
    (harness/record-failure! state
                             :nemesis
                             {:operation :partition}
                             first-throwable)
    (is (false? (harness/record-failure!
                 state
                 :nemesis
                 {:phase :teardown
                  :operation :heal}
                 later-throwable)))
    (is (false? (harness/record-failure!
                 state
                 :nemesis
                 {:phase :teardown
                  :operation :resume}
                 last-throwable)))
    (let [failure (harness/primary-failure state)]
      (is (= :nemesis (:source failure)))
      (is (= {:operation :partition} (:context failure)))
      (is (identical? first-throwable (:throwable failure))))
    (let [[later last] (harness/secondary-failures state)]
      (is (= {:phase :teardown
              :operation :heal}
             (:context later)))
      (is (identical? later-throwable (:throwable later)))
      (is (= {:phase :teardown
              :operation :resume}
             (:context last)))
      (is (identical? last-throwable (:throwable last))))))

(deftest records-concurrent-failures-once
  (let [state (harness/failure-state)
        start (promise)
        errors (mapv #(RuntimeException. (str "failure " %)) (range 16))
        workers (mapv (fn [id throwable]
                        (future
                          @start
                          (harness/record-failure! state
                                                   :nemesis
                                                   {:id id}
                                                   throwable)))
                      (range 16)
                      errors)]
    (deliver start true)
    (let [results (mapv deref workers)
          primary (harness/primary-failure state)
          recorded (cons primary (harness/secondary-failures state))]
      (is (= 1 (count (filter true? results))))
      (is (= (count errors) (count recorded)))
      (is (= (set (range 16)) (set (map (comp :id :context) recorded))))
      (is (= (set errors) (set (map :throwable recorded)))))))

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
      (let [state (harness/failure-state)
            primary (RuntimeException. "primary")]
        (is (false? (harness/record-failure!
                     state
                     :client
                     {:operation :read}
                     throwable)))
        (is (nil? (harness/primary-failure state)))
        (is (empty? (harness/secondary-failures state)))
        (harness/record-failure! state :client {:operation :write} primary)
        (is (false? (harness/record-failure!
                     state
                     :nemesis
                     {:phase :teardown}
                     throwable)))
        (is (identical? primary
                        (:throwable (harness/primary-failure state))))
        (is (empty? (harness/secondary-failures state)))))))

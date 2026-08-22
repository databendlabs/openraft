(ns jepsen.openraft.await-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.await :as await]))

(deftest retries-only-owned-unsatisfied-conditions
  (testing "an owned retry can later succeed"
    (let [attempts (atom 0)
          result (await/until!
                  :test-condition
                  #(if (= 1 (swap! attempts inc))
                     (await/retry! :test-condition {})
                     :ready)
                  {:retry-interval 0
                   :timeout 100})]
      (is (= :ready result))
      (is (= 2 @attempts))))

  (testing "an unknown exception escapes immediately by identity"
    (let [attempts (atom 0)
          error (RuntimeException. "wait bug")
          thrown (try
                   (await/until! :test-condition
                                 #(do
                                    (swap! attempts inc)
                                    (throw error))
                                 {:retry-interval 0
                                  :timeout 100})
                   nil
                   (catch Exception e
                     e))]
      (is (identical? error thrown))
      (is (= 1 @attempts))))

  (testing "only the owned condition matches a modeled timeout"
    (let [error (try
                  (await/until! :test-condition
                                #(await/retry! :test-condition {})
                                {:retry-interval 0
                                 :timeout 0})
                  nil
                  (catch Exception e
                    e))]
      (is (await/condition-timeout? error :test-condition))
      (is (await/condition-timeout? error))
      (is (false? (await/condition-timeout? error :other-condition))))))

(deftest preserves-wait-interruptions
  (doseq [[label make-error]
          [[:interrupted #(InterruptedException. "interrupted")]
           [:interrupted-io
            #(java.io.InterruptedIOException. "interrupted")]
           [:closed-by-interrupt
            #(java.nio.channels.ClosedByInterruptException.)]
           [:wrapped #(ex-info "interrupted" {:kind :interrupted})]]]
    (testing (name label)
      (Thread/interrupted)
      (try
        (let [error (make-error)
              [thrown interrupted?]
              (try
                (await/until! :test-condition
                              #(throw error)
                              {:retry-interval 0
                               :timeout 100})
                [nil (.isInterrupted (Thread/currentThread))]
                (catch Exception e
                  [e (.isInterrupted (Thread/currentThread))]))]
          (is (identical? error thrown))
          (is interrupted?))
        (finally
          (Thread/interrupted))))))

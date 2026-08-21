(ns jepsen.openraft.worker-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [client :as client]
             [nemesis :as nemesis]]
            [jepsen.openraft [harness :as harness]
             [worker :as worker]]))

(defn- test-client
  [{:keys [open-f invoke-f close-f]
    :or {open-f (fn [this _test _node] this)
         invoke-f (fn [_test op] (assoc op :type :ok))
         close-f (fn [_test])}}]
  (reify client/Client
    (open! [this test node]
      (open-f this test node))

    (setup! [this _test]
      this)

    (invoke! [_ test op]
      (invoke-f test op))

    (teardown! [_ _test])

    (close! [_ test]
      (close-f test))))

(defn- test-nemesis [invoke-f]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)

    (invoke! [_ test op]
      (invoke-f test op))

    (teardown! [_ _test])))

(defn- thrown-by [f]
  (try
    (f)
    nil
    (catch Throwable throwable
      throwable)))

(defn- assert-primary-failure
  [failure-state source context throwable]
  (let [failure (harness/primary-failure failure-state)]
    (is (= source (:source failure)))
    (is (= context (:context failure)))
    (is (identical? throwable (:throwable failure)))))

(deftest records-client-worker-failures
  (testing "open"
    (let [failure-state (harness/failure-state)
          throwable (RuntimeException. "open failed")
          subject (worker/wrap-client
                   failure-state
                   (test-client {:open-f (fn [& _] (throw throwable))}))
          thrown (thrown-by #(client/open! subject {} "n1"))]
      (is (identical? throwable thrown))
      (assert-primary-failure failure-state
                              :client
                              {:phase :open
                               :node "n1"}
                              throwable)))

  (testing "invoke"
    (let [failure-state (harness/failure-state)
          throwable (RuntimeException. "invoke failed")
          op {:type :invoke
              :process 0
              :f :read}
          subject (worker/wrap-client
                   failure-state
                   (test-client {:invoke-f (fn [& _] (throw throwable))}))
          opened (client/open! subject {} "n1")
          thrown (thrown-by #(client/invoke! opened {} op))]
      (is (identical? throwable thrown))
      (assert-primary-failure failure-state
                              :client
                              {:phase :invoke
                               :operation op}
                              throwable)))

  (testing "close"
    (let [failure-state (harness/failure-state)
          throwable (RuntimeException. "close failed")
          subject (worker/wrap-client
                   failure-state
                   (test-client {:close-f (fn [& _] (throw throwable))}))
          thrown (thrown-by #(client/close! subject {}))]
      (is (identical? throwable thrown))
      (assert-primary-failure failure-state
                              :client
                              {:phase :close}
                              throwable))))

(deftest records-nemesis-worker-failures
  (let [failure-state (harness/failure-state)
        throwable (RuntimeException. "nemesis failed")
        op {:type :info
            :process :nemesis
            :f :start-partition}
        subject (worker/wrap-nemesis
                 failure-state
                 (test-nemesis (fn [& _] (throw throwable))))
        setup-subject (nemesis/setup! subject {})
        thrown (thrown-by #(nemesis/invoke! setup-subject {} op))]
    (is (identical? throwable thrown))
    (assert-primary-failure failure-state
                            :nemesis
                            {:phase :invoke
                             :operation op}
                            throwable)))

(deftest modeled-outcomes-do-not-record-failures
  (let [failure-state (harness/failure-state)
        client-outcome {:type :info
                        :process 0
                        :f :write
                        :error :timeout}
        nemesis-outcome {:type :info
                         :process :nemesis
                         :f :start-partition
                         :value :no-supported-leader}
        client-subject (worker/wrap-client
                        failure-state
                        (test-client {:invoke-f (fn [& _] client-outcome)}))
        nemesis-subject (worker/wrap-nemesis
                         failure-state
                         (test-nemesis (fn [& _] nemesis-outcome)))]
    (is (= client-outcome
           (client/invoke! (client/open! client-subject {} "n1")
                           {}
                           client-outcome)))
    (is (= nemesis-outcome
           (nemesis/invoke! (nemesis/setup! nemesis-subject {})
                            {}
                            nemesis-outcome)))
    (is (nil? (harness/primary-failure failure-state)))))

(deftest interruptions-propagate-without-recording
  (doseq [[description throwable invoke]
          [["client"
            (InterruptedException. "interrupted")
            (fn [failure-state throwable]
              (let [subject (worker/wrap-client
                             failure-state
                             (test-client
                              {:invoke-f (fn [& _] (throw throwable))}))]
                (client/invoke! (client/open! subject {} "n1")
                                {}
                                {:type :invoke :f :read})))]
           ["nemesis"
            (ex-info "interrupted" {:kind :interrupted})
            (fn [failure-state throwable]
              (let [subject (worker/wrap-nemesis
                             failure-state
                             (test-nemesis
                              (fn [& _] (throw throwable))))]
                (nemesis/invoke! (nemesis/setup! subject {})
                                 {}
                                 {:type :info :f :start-partition})))]]]
    (testing description
      (let [failure-state (harness/failure-state)
            thrown (thrown-by #(invoke failure-state throwable))]
        (is (identical? throwable thrown))
        (is (nil? (harness/primary-failure failure-state)))))))

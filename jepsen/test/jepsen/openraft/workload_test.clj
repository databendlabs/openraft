(ns jepsen.openraft.workload-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.client :as client]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.workload :as workload]
            [knossos.model :as model]))

(defn- kv-client []
  (workload/->KVClient nil
                       (atom "n1:21001")
                       ["n1:21001"]
                       (atom nil)))

(deftest classifies-timeouts-by-operation
  (let [timeout (ex-info "timeout" {:kind :request-timeout})]
    (with-redefs [http/write! (fn [& _] (throw timeout))
                  http/linearizable-read! (fn [& _] (throw timeout))]
      (testing "a write timeout is indeterminate"
        (let [op {:type :invoke :f :write :value "value"}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (= :timeout (:error result)))))

      (testing "a read timeout has no state-machine effect"
        (let [op {:type :invoke :f :read :value nil}
              result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= :timeout (:error result)))))

      (testing "a final recovery write must complete"
        (let [op {:type :invoke
                  :f :write
                  :value "recovery-value"
                  :final? true}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (:unexpected? result))
          (is (= :request-timeout
                 (-> result :exception-data :kind))))))))

(deftest classifies-cas-outcomes
  (let [op {:type :invoke
            :f :cas
            :value ["old" "new"]
            :expected-version 1}]
    (testing "a successful CAS keeps its logical model value"
      (let [request (atom nil)
            versioned {:value "new" :version 2}]
        (with-redefs [http/cas! (fn [_endpoint _key expected-version new-value]
                                  (reset! request [expected-version new-value])
                                  versioned)]
          (let [result (client/invoke! (kv-client) {} op)]
            (is (= :ok (:type result)))
            (is (= ["old" "new"] (:value result)))
            (is (= [1 "new"] @request))))))

    (testing "a version mismatch is a definite failure"
      (with-redefs [http/cas! (fn [& _] nil)]
        (let [result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= :version-mismatch (:error result)))
          (is (= (:value op) (:value result))))))

    (testing "a CAS timeout is indeterminate"
      (let [timeout (ex-info "timeout" {:kind :request-timeout})]
        (with-redefs [http/cas! (fn [& _] (throw timeout))]
          (let [result (client/invoke! (kv-client) {} op)]
            (is (= :info (:type result)))
            (is (= :timeout (:error result)))))))))

(deftest keeps-versioned-values-out-of-model-history
  (let [client (kv-client)
        versioned {:value "value" :version 1}]
    (testing "writes keep their logical invocation value"
      (with-redefs [http/write! (fn [& _] versioned)]
        (let [op {:type :invoke :f :write :value "value"}
              result (client/invoke! client {} op)]
          (is (= :ok (:type result)))
          (is (= "value" (:value result))))))

    (testing "reads expose only the logical value"
      (with-redefs [http/linearizable-read! (fn [& _] versioned)]
        (let [op {:type :invoke :f :read :value nil}
              result (client/invoke! client {} op)]
          (is (= :ok (:type result)))
          (is (= "value" (:value result))))))))

(deftest timed-out-write-can-explain-a-later-read
  (let [history [{:process 0
                  :type :invoke
                  :f :write
                  :value "value"}
                 {:process 0
                  :type :info
                  :f :write
                  :value "value"
                  :error :timeout}
                 {:process 1
                  :type :invoke
                  :f :read
                  :value nil}
                 {:process 1
                  :type :ok
                  :f :read
                  :value "value"}]
        result (checker/check
                 (checker/linearizable
                   {:model (model/cas-register)})
                 {}
                 history
                 {})]
    (is (:valid? result)
        "an indeterminate write may have produced the value read later")))

(deftest generates-cas-only-with-a-known-version
  (testing "an unknown version produces a write"
    (let [generate (#'workload/cas-op (atom nil) (atom 0))
          op (generate nil nil)]
      (is (= :write (:f op)))
      (is (= "value-1" (:value op)))
      (is (not (contains? op :expected-version)))))

  (testing "a known version is captured separately from the model value"
    (let [latest-value (atom {:value "old" :version 7})
          generate (#'workload/cas-op latest-value (atom 0))
          op (generate nil nil)]
      (is (= :cas (:f op)))
      (is (= ["old" "value-1"] (:value op)))
      (is (= 7 (:expected-version op))))))

(deftest classifies-server-errors
  (testing "a read without quorum is a definite failure"
    (let [quorum-error (ex-info "no quorum"
                                {:kind :openraft-error
                                 :error {:QuorumNotEnough {}}})]
      (with-redefs [http/linearizable-read! (fn [& _]
                                              (throw quorum-error))]
        (let [op {:type :invoke :f :read :value nil}
              result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= :quorum-not-enough (:error result)))
          (is (not (:unexpected? result)))))))

  (testing "HTTP 400 is fail because app-http rejects it before handler execution"
    (let [bad-request (ex-info "bad request"
                               {:kind :http-error
                                :status 400})]
      (with-redefs [http/write! (fn [& _]
                                  (throw bad-request))]
        (let [op {:type :invoke :f :write :value "value"}
              result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= [:http 400] (:error result)))
          (is (:unexpected? result))
          (is (= "bad request" (:exception-message result)))
          (is (= 400 (get-in result [:exception-data :status])))))))

  (testing "HTTP 500 is info because response serialization follows handler execution"
    (let [server-error (ex-info "server error"
                                {:kind :http-error
                                 :status 500})]
      (with-redefs [http/write! (fn [& _]
                                  (throw server-error))]
        (let [op {:type :invoke :f :write :value "value"}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (= [:http 500] (:error result)))
          (is (:unexpected? result))
          (is (= "server error" (:exception-message result)))
          (is (= 500 (get-in result [:exception-data :status]))))))))

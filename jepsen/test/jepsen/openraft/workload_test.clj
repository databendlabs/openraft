(ns jepsen.openraft.workload-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.client :as client]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.workload :as workload]))

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
          (is (= :timeout (:error result))))))))

(deftest classifies-cas-outcomes
  (let [expected {:value "old" :version 1}
        op {:type :invoke
            :f :cas
            :value [expected "new"]}]
    (testing "a version mismatch is a definite failure"
      (with-redefs [http/cas! (fn [& _] nil)]
        (let [result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= (:value op) (:value result))))))

    (testing "a CAS timeout is indeterminate"
      (let [timeout (ex-info "timeout" {:kind :request-timeout})]
        (with-redefs [http/cas! (fn [& _] (throw timeout))]
          (let [result (client/invoke! (kv-client) {} op)]
            (is (= :info (:type result)))
            (is (= :timeout (:error result)))))))))

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
          (is (:unexpected? result))))))

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
          (is (:unexpected? result)))))))

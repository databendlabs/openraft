(ns jepsen.openraft.workload-test
  (:require [clojure.java.io :as io]
            [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.client :as client]
            [jepsen.generator :as gen]
            [jepsen.generator.test :as gen-test]
            [jepsen.history :as history]
            [jepsen.independent :as independent]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.workload :as workload]
            [jepsen.random :as random]
            [jepsen.store :as store]))

(def test-key "key-1")
(def other-key "key-2")

(defn- keyed
  ([value]
   (keyed test-key value))
  ([k value]
   (independent/tuple k value)))

(defn- check-in-temp-store [subject ops]
  (let [^java.io.File temp-dir
        (.toFile
         (java.nio.file.Files/createTempDirectory
          "openraft-workload-checker"
          (make-array java.nio.file.attribute.FileAttribute 0)))
        test {:name "workload-checker-test"
              :start-time "run"}]
    (try
      (with-redefs [store/base-dir (.getPath temp-dir)]
        (checker/check subject test (history/history ops) {}))
      (finally
        (doseq [file (reverse (file-seq temp-dir))]
          (io/delete-file file true))))))

(defn- kv-client []
  (workload/->KVClient nil
                       (atom "n1:21001")
                       ["n1:21001"]
                       (atom {})))

(deftest classifies-timeouts-by-operation
  (let [timeout (ex-info "timeout" {:kind :request-timeout})]
    (with-redefs [http/write! (fn [& _] (throw timeout))
                  http/linearizable-read! (fn [& _] (throw timeout))]
      (testing "a write timeout is indeterminate"
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (= :timeout (:error result)))))

      (testing "a read timeout has no state-machine effect"
        (let [op {:type :invoke :f :read :value (keyed nil)}
              result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= :timeout (:error result)))))

      (testing "a final recovery write must complete"
        (let [op {:type :invoke
                  :f :write
                  :value (keyed "recovery-value")
                  :final? true}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (:unexpected? result))
          (is (= :request-timeout
                 (-> result :exception-data :kind))))))))

(deftest classifies-cas-outcomes
  (let [op {:type :invoke
            :f :cas
            :value (keyed ["old" "new"])
            :expected-version 1}]
    (testing "a successful CAS keeps its logical model value"
      (let [request (atom nil)
            versioned {:value "new" :version 2}]
        (with-redefs [http/cas! (fn [_endpoint key expected-version new-value]
                                  (reset! request
                                          [key expected-version new-value])
                                  versioned)]
          (let [result (client/invoke! (kv-client) {} op)]
            (is (= :ok (:type result)))
            (is (= ["old" "new"] (val (:value result))))
            (is (= [test-key 1 "new"] @request))))))

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
        request (atom nil)
        versioned {:value "value" :version 1}]
    (testing "writes keep their logical invocation value"
      (with-redefs [http/write! (fn [_endpoint k value]
                                  (reset! request [k value])
                                  versioned)]
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! client {} op)]
          (is (= :ok (:type result)))
          (is (= [test-key "value"] @request))
          (is (= "value" (val (:value result)))))))

    (testing "reads expose only the logical value"
      (with-redefs [http/linearizable-read! (fn [_endpoint k]
                                              (reset! request [k])
                                              versioned)]
        (let [op {:type :invoke :f :read :value (keyed nil)}
              result (client/invoke! client {} op)]
          (is (= :ok (:type result)))
          (is (= [test-key] @request))
          (is (= test-key (key (:value result))))
          (is (= "value" (val (:value result)))))))))

(deftest timed-out-write-can-explain-a-later-read
  (let [history [{:process 0
                  :type :invoke
                  :f :write
                  :value (keyed "value")}
                 {:process 0
                  :type :info
                  :f :write
                  :value (keyed "value")
                  :error :timeout}
                 {:process 1
                  :type :invoke
                  :f :read
                  :value (keyed nil)}
                 {:process 1
                  :type :ok
                  :f :read
                  :value (keyed "value")}]
        subject (get-in (workload/workload {})
                        [:checker :checkers :linearizable])
        result (check-in-temp-store subject history)]
    (is (:valid? result)
        "an indeterminate write may have produced the value read later")))

(deftest checks-registers-independently
  (let [ops [{:process 0
              :type :invoke
              :f :write
              :value (keyed test-key "a")}
             {:process 0
              :type :ok
              :f :write
              :value (keyed test-key "a")}
             {:process 1
              :type :invoke
              :f :read
              :value (keyed test-key nil)}
             {:process 1
              :type :ok
              :f :read
              :value (keyed test-key "a")}
             {:process 0
              :type :invoke
              :f :write
              :value (keyed other-key "x")}
             {:process 0
              :type :ok
              :f :write
              :value (keyed other-key "x")}
             {:process 1
              :type :invoke
              :f :read
              :value (keyed other-key nil)}
             {:process 1
              :type :ok
              :f :read
              :value (keyed other-key "not-x")}]
        subject (get-in (workload/workload {})
                        [:checker :checkers :linearizable])
        result (check-in-temp-store subject ops)]
    (is (false? (:valid? result)))
    (is (= [other-key] (:failures result)))
    (is (true? (get-in result [:results test-key :valid?])))
    (is (false? (get-in result [:results other-key :valid?])))))

(deftest generates-multiple-independent-registers
  (let [invocations
        (random/with-seed 41
          (->> (gen-test/simulate
                (gen/limit 20 (:generator (workload/workload {})))
                (fn [_test op]
                  (assoc op :type :ok)))
               (filter #(= :invoke (:type %)))
               vec))
        bootstrap (take 5 invocations)
        main (drop 5 invocations)
        bootstrap-keys (set (map (comp key :value) bootstrap))
        main-keys (set (map (comp key :value) main))]
    (is (= (repeat 5 [:bootstrap :write])
           (map (juxt :phase :f) bootstrap)))
    (is (= 5 (count bootstrap-keys)))
    (is (every? #(independent/tuple? (:value %)) invocations))
    (is (< 1 (count main-keys)))
    (is (every? bootstrap-keys main-keys))))

(deftest tags-final-recovery-operations
  (let [write ((#'workload/final-write-op test-key (atom 0)) nil nil)
        read ((#'workload/final-read-op test-key) nil nil)]
    (is (= :final (:phase write)))
    (is (= :final (:phase read)))))

(deftest generates-cas-only-with-a-known-version
  (testing "a version observed for another key still produces a write"
    (let [latest-values (atom {other-key {:value "other" :version 6}})
          generate (#'workload/cas-op
                    [test-key]
                    latest-values
                    (atom 0))
          op (generate nil nil)]
      (is (= :write (:f op)))
      (is (= :main (:phase op)))
      (is (= test-key (key (:value op))))
      (is (= "value-1" (val (:value op))))
      (is (not (contains? op :expected-version)))))

  (testing "a known version is captured separately from the model value"
    (let [latest-values (atom {test-key {:value "old" :version 7}})
          generate (#'workload/cas-op
                    [test-key]
                    latest-values
                    (atom 0))
          op (generate nil nil)]
      (is (= :cas (:f op)))
      (is (= :main (:phase op)))
      (is (= test-key (key (:value op))))
      (is (= ["old" "value-1"] (val (:value op))))
      (is (= 7 (:expected-version op))))))

(deftest classifies-server-errors
  (testing "a read without quorum is a definite failure"
    (let [quorum-error (ex-info "no quorum"
                                {:kind :openraft-error
                                 :error {:QuorumNotEnough {}}})]
      (with-redefs [http/linearizable-read! (fn [& _]
                                              (throw quorum-error))]
        (let [op {:type :invoke :f :read :value (keyed nil)}
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
        (let [op {:type :invoke :f :write :value (keyed "value")}
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
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (= [:http 500] (:error result)))
          (is (:unexpected? result))
          (is (= "server error" (:exception-message result)))
          (is (= 500 (get-in result [:exception-data :status]))))))))

(deftest rethrows-interruptions
  (let [op {:type :invoke :f :write :value (keyed "value")}]
    (testing "a wrapped interruption from send! propagates instead of completing"
      (with-redefs [http/write! (fn [& _]
                                  (throw (ex-info "interrupted"
                                                  {:kind :interrupted})))]
        (is (thrown? clojure.lang.ExceptionInfo
                     (client/invoke! (kv-client) {} op)))))

    (testing "a raw InterruptedException propagates and restores the interrupt flag"
      (with-redefs [http/write! (fn [& _]
                                  (throw (InterruptedException. "boom")))]
        ;; Capture the exception and interrupt flag before any assertion runs:
        ;; clojure.test's report machinery uses dosync, which clears the flag.
        (Thread/interrupted)
        (let [thrown (try
                       (client/invoke! (kv-client) {} op)
                       nil
                       (catch InterruptedException e e))
              interrupted (Thread/interrupted)]
          (is (instance? InterruptedException thrown))
          (is interrupted
              "the interrupt flag is restored so Jepsen's control signals survive"))))))

(deftest unexpected-errors-checker-flags-tagged-operations
  (let [chk (#'workload/unexpected-errors-checker)]
    (testing "a history without unexpected errors is valid"
      (let [result (checker/check chk {} [{:type :ok :f :read}] {})]
        (is (:valid? result))
        (is (= 0 (:count result)))
        (is (= [] (:errors result)))))

    (testing "unexpected-tagged operations fail the check and are surfaced"
      (let [tagged {:type :info :f :write :error [:http 500] :unexpected? true}
            history [{:type :ok :f :read} tagged]
            result (checker/check chk {} history {})]
        (is (not (:valid? result)))
        (is (= 1 (:count result)))
        (is (= [tagged] (:errors result)))))))

(deftest requires-successful-main-phase-operations
  (let [chk (#'workload/meaningful-operations-checker)
        history [{:type :invoke :f :write :phase :bootstrap}
                 {:type :ok :f :write :phase :bootstrap}
                 {:type :invoke :f :read :phase :main}
                 {:type :ok :f :read :phase :main}
                 {:type :invoke :f :write :phase :main}
                 {:type :ok :f :write :phase :main}
                 {:type :invoke :f :cas :phase :main}
                 {:type :ok :f :cas :phase :main}
                 {:type :invoke :f :write :phase :final}
                 {:type :ok :f :write :phase :final}
                 {:type :invoke :f :read :phase :final}
                 {:type :ok :f :read :phase :final}]]
    (testing "all required main operations succeeded"
      (let [result (checker/check chk {} history {})]
        (is (true? (:valid? result)))
        (is (empty? (:missing-ok result)))
        (is (= 1 (get-in result [:counts :main :read :ok])))
        (is (= 1 (get-in result [:counts :main :write :ok])))
        (is (= 1 (get-in result [:counts :main :cas :ok])))
        (is (= 1 (get-in result [:counts :bootstrap :write :ok])))
        (is (= 1 (get-in result [:counts :final :write :ok])))))

    (testing "non-ok results cannot be hidden by bootstrap or final operations"
      (doseq [[f completion-type] [[:read :fail]
                                   [:write :info]
                                   [:cas :fail]]]
        (let [incomplete-history
              (mapv (fn [op]
                      (if (and (= :main (:phase op))
                               (= f (:f op))
                               (= :ok (:type op)))
                        (assoc op :type completion-type)
                        op))
                    history)
              result (checker/check chk {} incomplete-history {})]
          (is (= :unknown (:valid? result)))
          (is (= [f] (:missing-ok result)))
          (is (= 0 (get-in result [:counts :main f :ok])))
          (is (= 1 (get-in result
                           [:counts :main f completion-type]))))))))

(deftest workload-checker-requires-meaningful-main-traffic
  (let [history [{:process 0
                  :type :invoke
                  :f :write
                  :phase :bootstrap
                  :value (keyed "initial")}
                 {:process 0
                  :type :ok
                  :f :write
                  :phase :bootstrap
                  :value (keyed "initial")}
                 {:process 1
                  :type :invoke
                  :f :read
                  :phase :main
                  :value (keyed nil)}
                 {:process 1
                  :type :ok
                  :f :read
                  :phase :main
                  :value (keyed "initial")}]
        chk (:checker (workload/workload {}))
        result (check-in-temp-store chk history)]
    (is (true? (get-in result [:linearizable :valid?])))
    (is (= :unknown
           (get-in result [:meaningful-operations :valid?])))
    (is (= [:write :cas]
           (get-in result [:meaningful-operations :missing-ok])))
    (is (= :unknown (:valid? result)))))

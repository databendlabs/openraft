(ns jepsen.openraft.workload-test
  (:require [cheshire.core :as json]
            [cheshire.parse :as json-parse]
            [clojure.java.io :as io]
            [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.client :as client]
            [jepsen.generator :as gen]
            [jepsen.generator.test :as gen-test]
            [jepsen.history :as history]
            [jepsen.independent :as independent]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.harness :as harness]
            [jepsen.openraft.worker :as worker]
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

(defn- kv-client
  ([]
   (kv-client (atom {})))
  ([latest-values]
   (workload/->KVClient nil
                        (atom "n1:21001")
                        ["n1:21001"]
                        latest-values)))

(defn- raw-http-response [body]
  {:method "POST"
   :uri "http://n1:21001/write"
   :status 200
   :body body})

(defn- http-response [body]
  (raw-http-response (json/generate-string body)))

(defn- invoke-with-response [subject op body]
  (with-redefs [http/post! (fn [& _] (http-response body))]
    (client/invoke! subject {} op)))

(deftest classifies-timeouts-by-operation
  (let [timeout (ex-info "timeout" {:kind :request-timeout})]
    (with-redefs [http/write! (fn [& _] (throw timeout))
                  http/linearizable-read! (fn [& _] (throw timeout))]
      (testing "a write timeout is indeterminate"
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (= :timeout (:error result)))
          (is (not (contains? result :unexpected-sut-response?)))))

      (testing "a read timeout has no state-machine effect"
        (let [op {:type :invoke :f :read :value (keyed nil)}
              result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= :timeout (:error result)))
          (is (not (contains? result :unexpected-sut-response?)))))

      (testing "a final recovery write must complete"
        (let [op {:type :invoke
                  :f :write
                  :value (keyed "recovery-value")
                  :final? true}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (not (contains? result :unexpected-sut-response?)))
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
            (is (= [test-key 1 "new"] @request))
            (is (not (contains? result :unexpected-sut-response?)))))))

    (testing "a version mismatch is a definite failure"
      (with-redefs [http/cas! (fn [& _] nil)]
        (let [result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= :version-mismatch (:error result)))
          (is (= (:value op) (:value result)))
          (is (not (contains? result :unexpected-sut-response?))))))

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

(deftest tracks-latest-values-independently
  (let [latest-values (atom {})
        newer {:value "newer" :version 2}
        stale {:value "stale" :version 1}
        other {:value "other" :version 3}]
    (#'workload/remember-latest! latest-values test-key newer)
    (#'workload/remember-latest! latest-values other-key other)
    (#'workload/remember-latest! latest-values test-key stale)
    (#'workload/remember-latest! latest-values test-key nil)
    (is (= {test-key newer
            other-key other}
           @latest-values))))

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
        subject (:checker (workload/workload {}))
        result (check-in-temp-store subject history)]
    (is (true? (get-in result [:linearizable :valid?]))
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
        subject (:checker (workload/workload {}))
        result (get (check-in-temp-store subject ops) :linearizable)]
    (is (false? (:valid? result)))
    (is (= [other-key] (:failures result)))
    (is (true? (get-in result [:results test-key :valid?])))
    (is (false? (get-in result [:results other-key :valid?])))))

(deftest generates-multiple-independent-registers
  (let [workload (workload/workload {})
        version (atom 0)
        values (atom {})
        cas-observations (atom [])
        store! (fn [k value]
                 (let [versioned {:value value
                                  :version (swap! version inc)}]
                   (swap! values assoc k versioned)
                   versioned))
        test {:nodes ["n1"]}
        subject (client/open! (:client workload) test "n1")
        invocations
        (with-redefs [http/write! (fn [_endpoint k value]
                                    (store! k value))
                      http/linearizable-read! (fn [_endpoint k]
                                                (get @values k))
                      http/cas! (fn [_endpoint k expected-version new-value]
                                  (when (= expected-version
                                           (:version (get @values k)))
                                    (store! k new-value)))]
          (random/with-seed 41
            (->> (gen-test/simulate
                  (gen/limit 20 (:generator workload))
                  (fn [_test op]
                    (when (and (= :invoke (:type op))
                               (= :cas (:f op)))
                      (let [k (key (:value op))
                            [expected-value _] (val (:value op))
                            current (get @values k)]
                        (swap! cas-observations conj
                               {:key k
                                :from-op [(:expected-version op)
                                          expected-value]
                                :from-store [(:version current)
                                             (:value current)]})))
                    (client/invoke! subject test op)))
                 (filter #(= :invoke (:type %)))
                 vec)))
        expected-keys (set @#'workload/key-names)
        bootstrap (take (count expected-keys) invocations)
        main (drop (count expected-keys) invocations)
        bootstrap-keys (set (map (comp key :value) bootstrap))
        main-keys (set (map (comp key :value) main))]
    (is (= (repeat (count expected-keys) [:bootstrap :write])
           (map (juxt :phase :f) bootstrap)))
    (is (= expected-keys bootstrap-keys main-keys))
    (is (= #{:main} (set (map :phase main))))
    (is (seq @cas-observations))
    (is (every? #(= (:from-op %) (:from-store %))
                @cas-observations))
    (is (every? #(independent/tuple? (:value %)) invocations))
    (client/close! subject test)))

(deftest final-recovery-checks-every-register
  (let [keys @#'workload/key-names
        invocations (->> (gen-test/simulate
                          (:final-generator (workload/workload {}))
                          (fn [_test op]
                            (assoc op :type :ok)))
                         (filter #(= :invoke (:type %))))]
    (is (= (concat (map #(vector :write :final true %) keys)
                   (map #(vector :read :final true %) keys))
           (map (juxt :f :phase :final? (comp key :value))
                invocations)))))

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
          (is (not (contains? result :unexpected-sut-response?)))))))

  (testing "HTTP 400 is fail because app-http rejects it before handler execution"
    (let [bad-request (ex-info "bad request"
                               {:kind :http-error
                                :method "POST"
                                :uri "http://n1:21001/write"
                                :status 400
                                :body "bad request"})]
      (with-redefs [http/write! (fn [& _]
                                  (throw bad-request))]
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! (kv-client) {} op)]
          (is (= :fail (:type result)))
          (is (= [:http 400] (:error result)))
          (is (true? (:unexpected-sut-response? result)))
          (is (= "bad request" (:exception-message result)))
          (is (= 400 (get-in result [:exception-data :status])))
          (is (= "bad request" (get-in result [:exception-data :body])))))))

  (testing "HTTP 500 is info because response serialization follows handler execution"
    (let [server-error (ex-info "server error"
                                {:kind :http-error
                                 :method "POST"
                                 :uri "http://n1:21001/write"
                                 :status 500
                                 :body "server error"})]
      (with-redefs [http/write! (fn [& _]
                                  (throw server-error))]
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (= [:http 500] (:error result)))
          (is (true? (:unexpected-sut-response? result)))
          (is (= "server error" (:exception-message result)))
          (is (= 500 (get-in result [:exception-data :status]))))))))

(deftest records-clearly-unexpected-sut-responses
  (testing "invalid JSON is a marked Outcome, not a Harness failure"
    (let [failure-state (harness/failure-state)
          response {:method "POST"
                    :uri "http://n1:21001/write"
                    :status 200
                    :body "not-json"}
          invalid-json (ex-info "invalid JSON"
                                {:kind :invalid-json
                                 :response response})
          subject (worker/wrap-client failure-state (kv-client))]
      (with-redefs [http/write! (fn [& _] (throw invalid-json))]
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! subject {} op)]
          (is (= :info (:type result)))
          (is (= :invalid-json (:error result)))
          (is (true? (:unexpected-sut-response? result)))
          (is (= response (get-in result [:exception-data :response])))
          (is (nil? (harness/primary-failure failure-state)))))))

  (testing "an unmodeled OpenRaft Err variant is marked"
    (let [error {:UnexpectedError {:message "unsupported"}}
          openraft-error (ex-info "unexpected OpenRaft error"
                                  {:kind :openraft-error
                                   :error error
                                   :response {:status 200
                                              :body "Err"}})]
      (with-redefs [http/write! (fn [& _] (throw openraft-error))]
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! (kv-client) {} op)]
          (is (= :info (:type result)))
          (is (= :openraft-error (:error result)))
          (is (true? (:unexpected-sut-response? result)))
          (is (= error (get-in result [:exception-data :error]))))))))

(deftest leaves-modeled-sut-outcomes-unmarked
  (doseq [[description throwable expected-type expected-error]
          [["connection failure"
            (ex-info "unreachable" {:kind :unreachable})
            :fail
            :unreachable]
           ["transport failure"
            (ex-info "transport" {:kind :transport-error})
            :info
            :transport-error]
           ["leader redirect without a target"
            (ex-info "not leader"
                     {:kind :openraft-error
                      :error {:ForwardToLeader {}}})
            :fail
            :not-leader]
           ["quorum failure"
            (ex-info "no quorum"
                     {:kind :openraft-error
                      :error {:QuorumNotEnough {}}})
            :fail
            :quorum-not-enough]]]
    (testing description
      (with-redefs [http/write! (fn [& _] (throw throwable))]
        (let [op {:type :invoke :f :write :value (keyed "value")}
              result (client/invoke! (kv-client) {} op)]
          (is (= expected-type (:type result)))
          (is (= expected-error (:error result)))
          (is (not (contains? result :unexpected-sut-response?))))))))

(deftest validates-http-200-application-responses
  (testing "a valid Ok payload completes successfully"
    (let [op {:type :invoke :f :write :value (keyed "value")}
          body {:Ok {:data {:value {:value "value" :version 1}}}}
          result (invoke-with-response (kv-client) op body)]
      (is (= :ok (:type result)))
      (is (not (contains? result :unexpected-sut-response?)))))

  (testing "a known Err remains a modeled Outcome"
    (let [op {:type :invoke :f :read :value (keyed nil)}
          body {:Err {:QuorumNotEnough {:cluster "n1,n2,n3"
                                        :got ["n1"]}}}
          result (invoke-with-response (kv-client) op body)]
      (is (= :fail (:type result)))
      (is (= :quorum-not-enough (:error result)))
      (is (not (contains? result :unexpected-sut-response?)))))

  (testing "invalid response unions are unexpected Outcomes"
    (doseq [[label body reason]
            [["missing arm" {} :missing-response-arm]
             ["ambiguous arms"
              {:Ok {:data {:value {:value "value" :version 1}}}
               :Err {:ForwardToLeader {}}}
              :ambiguous-response-arms]
             ["unknown arm" {:Result {}} :unknown-response-arm]]]
      (testing label
        (let [failure-state (harness/failure-state)
              subject (worker/wrap-client failure-state (kv-client))
              op {:type :invoke :f :write :value (keyed "value")}
              result (invoke-with-response subject op body)]
          (is (= :info (:type result)))
          (is (= :invalid-response (:error result)))
          (is (true? (:unexpected-sut-response? result)))
          (is (= reason (get-in result [:exception-data :reason])))
          (is (= (json/generate-string body)
                 (get-in result
                         [:exception-data :response :body-preview])))
          (is (= (count (json/generate-string body))
                 (get-in result
                         [:exception-data
                          :response
                          :body-character-count])))
          (is (nil? (harness/primary-failure failure-state)))))))

  (testing "malformed successful payloads are unexpected Outcomes"
    (doseq [[label op body expected-type path]
            [["write nil"
              {:type :invoke :f :write :value (keyed "value")}
              {:Ok {:data {:value nil}}}
              :info
              [:data :value]]
             ["CAS missing value"
              {:type :invoke
               :f :cas
               :value (keyed ["old" "new"])
               :expected-version 1}
              {:Ok {:data {}}}
              :info
              [:data :value]]
             ["read malformed value"
              {:type :invoke :f :read :value (keyed nil)}
              {:Ok {:value {:value 1 :version 2}}}
              :fail
              [:value]]]]
      (testing label
        (let [result (invoke-with-response (kv-client) op body)]
          (is (= expected-type (:type result)))
          (is (= :invalid-response (:error result)))
          (is (true? (:unexpected-sut-response? result)))
          (is (= path (get-in result [:exception-data :payload-path])))))))

  (testing "malformed modeled Err payloads are unexpected Outcomes"
    (doseq [[label op body expected-type variant]
            [["ForwardToLeader payload"
              {:type :invoke :f :write :value (keyed "value")}
              {:Err {:ForwardToLeader "not-an-object"}}
              :info
              :ForwardToLeader]
             ["QuorumNotEnough payload"
              {:type :invoke :f :read :value (keyed nil)}
              {:Err {:QuorumNotEnough ["not" "an" "object"]}}
              :fail
              :QuorumNotEnough]]]
      (testing label
        (let [failure-state (harness/failure-state)
              subject (worker/wrap-client failure-state (kv-client))
              result (invoke-with-response subject op body)]
          (is (= expected-type (:type result)))
          (is (= :invalid-response (:error result)))
          (is (true? (:unexpected-sut-response? result)))
          (is (= :invalid-error-payload
                 (get-in result [:exception-data :reason])))
          (is (= variant
                 (get-in result [:exception-data :error-variant])))
          (is (nil? (harness/primary-failure failure-state))))))))

(deftest rejects-invalid-json-documents-as-workload-outcomes
  (let [failure-state (harness/failure-state)
        subject (worker/wrap-client failure-state (kv-client))
        op {:type :invoke :f :write :value (keyed "value")}
        response (raw-http-response
                  (str "{\"Ok\":{\"data\":{\"value\":"
                       "{\"value\":\"value\",\"version\":1}}},"
                       "\"Ok\":{\"data\":{\"value\":"
                       "{\"value\":\"other\",\"version\":2}}}}"))
        result (with-redefs [http/post! (fn [& _] response)]
                 (client/invoke! subject {} op))]
    (is (= :info (:type result)))
    (is (= :invalid-json (:error result)))
    (is (true? (:unexpected-sut-response? result)))
    (is (= (count (:body response))
           (get-in result
                   [:exception-data :response :body-character-count])))
    (is (nil? (harness/primary-failure failure-state)))))

(deftest unexpected-mutation-responses-do-not-poison-latest-values
  (let [original {test-key {:value "old" :version 1}}
        latest-values (atom original)
        failure-state (harness/failure-state)
        subject (worker/wrap-client failure-state
                                    (kv-client latest-values))]
    (doseq [[label op body reason]
            [["write"
              {:type :invoke :f :write :value (keyed "value")}
              {:Ok {:data {:value {:value "other" :version 2}}}}
              :unexpected-payload-value]
             ["CAS value"
              {:type :invoke
               :f :cas
               :value (keyed ["old" "new"])
               :expected-version 1}
              {:Ok {:data {:value {:value "other" :version 2}}}}
              :unexpected-payload-value]
             ["CAS version"
              {:type :invoke
               :f :cas
               :value (keyed ["old" "new"])
               :expected-version 1}
              {:Ok {:data {:value {:value "new" :version 1}}}}
              :non-increasing-cas-version]]]
      (testing label
        (let [result (invoke-with-response subject op body)]
          (is (= :info (:type result)))
          (is (= :invalid-response (:error result)))
          (is (true? (:unexpected-sut-response? result)))
          (is (= reason
                 (get-in result [:exception-data :reason])))
          (is (= original @latest-values)))))
    (is (nil? (harness/primary-failure failure-state)))))

(deftest rethrows-interruptions
  (doseq [[label throwable]
          [["InterruptedException"
            (InterruptedException. "interrupted")]
           ["InterruptedIOException"
            (java.io.InterruptedIOException. "interrupted")]
           ["ClosedByInterruptException"
            (java.nio.channels.ClosedByInterruptException.)]
           ["tagged interruption"
            (ex-info "interrupted" {:kind :interrupted})]]]
    (testing label
      ;; Capture the exception and interrupt flag before any assertion runs:
      ;; clojure.test's report machinery uses dosync, which clears the flag.
      (Thread/interrupted)
      (let [failure-state (harness/failure-state)
            subject (worker/wrap-client failure-state (kv-client))
            op {:type :invoke :f :write :value (keyed "value")}
            thrown (with-redefs [http/write! (fn [& _] (throw throwable))]
                     (try
                       (client/invoke! subject {} op)
                       nil
                       (catch Throwable e
                         e)))
            interrupted (Thread/interrupted)]
        (is (identical? throwable thrown))
        (is interrupted
            "the interrupt flag is restored so Jepsen's control signals survive")
        (is (nil? (harness/primary-failure failure-state)))))))

(deftest rethrows-unknown-client-exceptions
  (let [op {:type :invoke :f :write :value (keyed "value")}
        throwable (RuntimeException. "client bug")]
    (with-redefs [http/write! (fn [& _] (throw throwable))]
      (let [thrown (try
                     (client/invoke! (kv-client) {} op)
                     nil
                     (catch RuntimeException e
                       e))]
        (is (identical? throwable thrown))))))

(deftest records-unknown-parser-exceptions-as-harness-failures
  (let [failure-state (harness/failure-state)
        subject (worker/wrap-client failure-state (kv-client))
        op {:type :invoke :f :write :value (keyed "value")}
        response (http-response
                  {:Ok {:data {:value {:value "value" :version 1}}}})
        throwable (RuntimeException. "parser bug")
        thrown (with-redefs [http/post! (fn [& _] response)
                             json-parse/parse-strict (fn [& _]
                                                       (throw throwable))]
                 (try
                   (client/invoke! subject {} op)
                   nil
                   (catch RuntimeException e
                     e)))
        failure (harness/primary-failure failure-state)]
    (is (identical? throwable thrown))
    (is (= :client (:source failure)))
    (is (= op (get-in failure [:context :operation])))
    (is (identical? throwable (:throwable failure)))))

(deftest unexpected-sut-responses-checker-aggregates-marked-outcomes
  (let [chk (#'workload/unexpected-sut-responses-checker)]
    (testing "a history without unexpected SUT responses is valid"
      (let [result (checker/check chk {} [{:type :ok :f :read}] {})]
        (is (true? (:valid? result)))
        (is (= 0 (:count result)))
        (is (= [] (:responses result)))))

    (testing "marked outcomes fail the check and are aggregated"
      (let [first-response {:type :info
                            :f :write
                            :error [:http 500]
                            :unexpected-sut-response? true}
            second-response {:type :fail
                             :f :read
                             :error :invalid-json
                             :unexpected-sut-response? true}
            history [{:type :ok :f :read}
                     first-response
                     {:type :info :f :write :error :timeout}
                     second-response]
            result (checker/check chk {} history {})]
        (is (false? (:valid? result)))
        (is (= 2 (:count result)))
        (is (= [first-response second-response]
               (:responses result)))))))

(deftest workload-checker-rejects-unsuccessful-final-operations
  (let [invocation {:process 0
                    :type :invoke
                    :f :write
                    :phase :final
                    :final? true
                    :value (keyed "value")}
        timeout (assoc invocation
                       :type :info
                       :error :timeout)
        chk (:checker (workload/workload {}))
        result (check-in-temp-store chk [invocation timeout])]
    (is (false? (:valid? result)))
    (is (false? (get-in result [:final-workload :valid?])))
    (is (= (select-keys timeout
                        [:process :type :f :phase :final? :value :error])
           (-> result
               (get-in [:final-workload :failures])
               first
               (select-keys
                [:process :type :f :phase :final? :value :error]))))
    (is (true? (get-in result [:unexpected-sut-responses :valid?])))
    (is (contains? result :linearizable))
    (is (contains? result :meaningful-operations))))

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
    (is (true? (get-in result [:final-workload :valid?])))
    (is (true? (get-in result [:unexpected-sut-responses :valid?])))
    (is (= :unknown
           (get-in result [:meaningful-operations :valid?])))
    (is (= [:write :cas]
           (get-in result [:meaningful-operations :missing-ok])))
    (is (= :unknown (:valid? result)))))

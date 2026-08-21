(ns jepsen.openraft.checker-test
  (:require [clojure.edn :as edn]
            [clojure.java.io :as io]
            [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.history :as history]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.harness :as harness]
            [jepsen.store :as store]))

(defn- result-checker [calls result]
  (reify checker/Checker
    (check [_ _test _history _opts]
      (swap! calls inc)
      result)))

(def ^:private escaped-worker-op
  {:type :info
   :f :read
   :exception {:via [{:type 'java.lang.IllegalStateException}]}})

(deftest records-the-random-seed
  (is (= {:valid? true
          :seed 41}
         (checker/check (openraft-checker/random-seed-checker)
                        {:seed 41}
                        (history/history [])
                        {}))))

(deftest reports-unhandled-worker-exceptions
  (let [subject (openraft-checker/strict-unhandled-exceptions)
        result (checker/check subject
                              {}
                              (history/history [escaped-worker-op])
                              {})
        exception (first (:exceptions result))]
    (is (false? (:valid? result)))
    (is (= {:count 1
            :class 'java.lang.IllegalStateException}
           (select-keys exception [:count :class])))
    (let [escaped (:escaped-worker-exceptions result)]
      (is (= (select-keys exception [:count :class])
             (select-keys (first escaped) [:count :class])))
      (is (= escaped (edn/read-string (pr-str escaped)))))
    (let [clean-result (checker/check subject
                                      {}
                                      (history/history [])
                                      {})]
      (is (true? (:valid? clean-result)))
      (is (not (contains? clean-result :escaped-worker-exceptions))))))

(deftest preserves-results-without-a-harness-failure
  (let [failure-state (harness/failure-state)
        calls (atom 0)
        expected {:valid? :unknown
                  :workload {:valid? true}
                  :nemesis {:valid? :unknown}}
        subject (openraft-checker/reject-harness-failures
                 failure-state
                 (result-checker calls expected))
        result (checker/check subject {} (history/history []) {})]
    (is (identical? expected result))
    (is (= 1 @calls))
    (is (not (contains? result :harness-failure)))))

(deftest rejects-recorded-harness-failures-after-analysis
  (doseq [delegate-verdict [true :unknown]]
    (testing (str "delegate verdict " delegate-verdict)
      (let [failure-state (harness/failure-state)
            primary (ex-info "teardown failed" {:unsafe (Object.)})
            secondary (RuntimeException. "logging failed")
            calls (atom 0)
            nested {:workload {:valid? false
                               :counterexample [{:f :write}]}
                    :nemesis {:valid? true
                              :observed-modes [:partition]}}
            delegate-result (assoc nested :valid? delegate-verdict)
            _ (harness/record-failure!
               failure-state
               :nemesis
               {:phase :teardown
                :component :partition}
               primary)
            _ (harness/record-failure!
               failure-state
               :nemesis
               {:phase :teardown
                :diagnostic :failure-log}
               secondary)
            result (checker/check
                    (openraft-checker/reject-harness-failures
                     failure-state
                     (result-checker calls delegate-result))
                    {}
                    (history/history [])
                    {})
            evidence (:harness-failure result)]
        (is (false? (:valid? result)))
        (is (= nested (select-keys result [:workload :nemesis])))
        (is (= 1 @calls))
        (is (= {:valid? false
                :primary
                {:source :nemesis
                 :context {:phase :teardown
                           :component :partition}
                 :exception
                 {:class "clojure.lang.ExceptionInfo"
                  :message "teardown failed"}}
                :secondary
                [{:source :nemesis
                  :context {:phase :teardown
                            :diagnostic :failure-log}
                  :exception
                  {:class "java.lang.RuntimeException"
                   :message "logging failed"}}]}
               evidence))
        (is (= evidence (edn/read-string (pr-str evidence))))))))

(deftest rejects-strict-fallback-without-mutating-runtime-state
  (let [failure-state (harness/failure-state)
        workload-result {:valid? true
                         :details {:reads 4}}
        aggregate (checker/compose
                   {:exceptions
                    (openraft-checker/strict-unhandled-exceptions)
                    :workload (result-checker (atom 0) workload-result)})
        result (checker/check
                (openraft-checker/reject-harness-failures
                 failure-state
                 aggregate
                 :exceptions)
                {}
                (history/history [escaped-worker-op])
                {})
        strict-result (:exceptions result)
        escaped (:escaped-worker-exceptions strict-result)]
    (is (false? (:valid? result)))
    (is (= workload-result (:workload result)))
    (is (false? (:valid? strict-result)))
    (is (seq escaped))
    (is (= {:valid? false
            :strict-fallback {:escaped-worker-exceptions escaped}}
           (:harness-failure result)))
    (is (nil? (harness/primary-failure failure-state)))
    (is (empty? (harness/secondary-failures failure-state)))))

(deftest preserves-direct-and-strict-harness-diagnostics
  (let [failure-state (harness/failure-state)
        primary (RuntimeException. "client failed")
        secondary (RuntimeException. "cleanup failed")
        workload-result {:valid? false
                         :counterexample [{:f :write :value 1}]}
        aggregate (checker/compose
                   {:exceptions
                    (openraft-checker/strict-unhandled-exceptions)
                    :workload (result-checker (atom 0) workload-result)})
        _ (harness/record-failure!
           failure-state :client {:operation :write} primary)
        _ (harness/record-failure!
           failure-state :nemesis {:phase :teardown} secondary)
        result (checker/check
                (openraft-checker/reject-harness-failures
                 failure-state
                 aggregate
                 :exceptions)
                {}
                (history/history [escaped-worker-op])
                {})
        strict-result (:exceptions result)
        evidence (:harness-failure result)]
    (is (false? (:valid? result)))
    (is (= workload-result (:workload result)))
    (is (= {:source :client
            :context {:operation :write}
            :exception {:class "java.lang.RuntimeException"
                        :message "client failed"}}
           (:primary evidence)))
    (is (= [{:source :nemesis
             :context {:phase :teardown}
             :exception {:class "java.lang.RuntimeException"
                         :message "cleanup failed"}}]
           (:secondary evidence)))
    (is (= {:escaped-worker-exceptions
            (:escaped-worker-exceptions strict-result)}
           (:strict-fallback evidence)))
    (is (false? (:valid? strict-result)))
    (is (= (select-keys (first (:exceptions strict-result)) [:count :class])
           (select-keys
            (first (:escaped-worker-exceptions strict-result))
            [:count :class])))
    (is (= evidence (edn/read-string (pr-str evidence))))))

(deftest does-not-reject-interruption
  (let [failure-state (harness/failure-state)
        expected {:valid? true}]
    (is (false? (harness/record-failure!
                 failure-state
                 :nemesis
                 {:phase :teardown}
                 (InterruptedException. "cancelled"))))
    (is (= expected
           (checker/check
            (openraft-checker/reject-harness-failures
             failure-state
             (result-checker (atom 0) expected))
            {}
            (history/history [])
            {})))))

(deftest checks-downloaded-logs-for-node-panics
  (let [^java.io.File temp-dir
        (.toFile
         (java.nio.file.Files/createTempDirectory
          "openraft-jepsen-checker"
          (make-array java.nio.file.attribute.FileAttribute 0)))
        test {:name "panic-checker-test"
              :start-time "run"
              :nodes ["n1" "n2"]}
        subject (openraft-checker/required-log-file-pattern
                 openraft-checker/node-panic-pattern
                 "openraft.log")]
    (try
      (with-redefs [store/base-dir (.getPath temp-dir)]
        (let [write-log! (fn [node content]
                           (let [file (store/path test node "openraft.log")]
                             (io/make-parents file)
                             (spit file content)))
              check #(checker/check subject test (history/history []) {})]
          (write-log! "n1" "INFO node started\n")
          (write-log! "n2" "INFO vote committed\n")
          (testing "clean logs pass"
            (is (= {:valid? true
                    :filename "openraft.log"
                    :missing-nodes []
                    :count 0
                    :matches []}
                   (check))))

          (write-log! "n2" "INFO before panic\nOPENRAFT_JEPSEN_PANIC\n")
          (testing "a panic marker fails the check"
            (is (= {:valid? false
                    :filename "openraft.log"
                    :missing-nodes []
                    :count 1
                    :matches [{:node "n2"
                               :line "OPENRAFT_JEPSEN_PANIC"}]}
                   (check))))

          (io/delete-file (store/path test "n2" "openraft.log"))
          (testing "a missing node log is indeterminate"
            (is (= {:valid? :unknown
                    :filename "openraft.log"
                    :missing-nodes ["n2"]
                    :count 0
                    :matches []}
                   (check))))

          (write-log! "n1" "INFO before panic\nOPENRAFT_JEPSEN_PANIC\n")
          (testing "a panic marker takes precedence over a missing log"
            (is (= {:valid? false
                    :filename "openraft.log"
                    :missing-nodes ["n2"]
                    :count 1
                    :matches [{:node "n1"
                               :line "OPENRAFT_JEPSEN_PANIC"}]}
                   (check))))))
      (finally
        (doseq [file (reverse (file-seq temp-dir))]
          (io/delete-file file true))))))

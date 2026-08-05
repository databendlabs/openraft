(ns jepsen.openraft.cli-test
  (:require [clojure.java.io :as io]
            [clojure.test :refer [deftest is testing]]
            [clojure.tools.cli :as tools-cli]
            [jepsen.checker :as checker]
            [jepsen.history :as history]
            [jepsen.openraft.cli :as cli]
            [jepsen.openraft.db :as openraft-db]
            [jepsen.random :as random]
            [jepsen.store :as store]))

(deftest records-and-applies-the-random-seed
  (testing "a generated seed is recorded and applied once"
    (let [applied-seeds (atom [])
          parsed (with-redefs [random/set-seed!
                               #(swap! applied-seeds conj %)]
                   (#'cli/ensure-random-seed {:options {}}))]
      (is (integer? (get-in parsed [:options :seed])))
      (is (= [(get-in parsed [:options :seed])]
             @applied-seeds))))

  (testing "test construction does not restart an explicit seed"
    (let [applied-seeds (atom [])
          opts {:nemesis :partition
                :seed 41
                :time-limit 10}]
      (with-redefs [random/set-seed! #(swap! applied-seeds conj %)]
        (let [parsed (#'cli/ensure-random-seed {:options opts})]
          (cli/openraft-test (:options parsed))
          (cli/openraft-test (:options parsed))))
      (is (= [41] @applied-seeds)))))

(deftest validates-the-random-seed
  (testing "a malformed seed is rejected"
    (let [parsed (tools-cli/parse-opts ["--seed" "12e5"] cli/cli-opts)]
      (is (some #(re-find #"Must be an integer" %)
                (:errors parsed)))))

  (testing "the seed remains optional"
    (is (empty? (:errors (tools-cli/parse-opts [] cli/cli-opts))))))

(deftest validates-the-snapshot-threshold
  (testing "the default applies snapshot pressure"
    (is (= openraft-db/default-snapshot-threshold
           (get-in (tools-cli/parse-opts [] cli/cli-opts)
                   [:options :snapshot-threshold]))))

  (testing "malformed and zero thresholds are rejected"
    (doseq [value ["abc" "0"]]
      (let [parsed (tools-cli/parse-opts ["--snapshot-threshold" value]
                                         cli/cli-opts)]
        (is (some #(re-find #"Must be a positive integer" %)
                  (:errors parsed)))))))

(deftest selects-composable-nemeses
  (testing "chaos is the default"
    (is (= [:partition :process :pause :membership]
           (#'cli/normalize-nemeses nil))))

  (testing "comma-separated faults are parsed and canonically ordered"
    (is (= [:partition :process]
           (#'cli/normalize-nemeses
            (#'cli/parse-nemeses "process, partition")))))

  (testing "chaos expands to every composable fault without duplicates"
    (is (= [:partition :process :pause :membership]
           (#'cli/normalize-nemeses [:chaos :partition]))))

  (testing "membership can be combined with another fault"
    (is (#'cli/valid-nemeses? [:membership :partition]))))

(deftest composes-selected-nemesis-checkers
  (let [test (cli/openraft-test {:nemesis [:membership :partition]
                                 :nodes ["n1" "n2" "n3" "n4" "n5"]
                                 :time-limit 10})
        checkers (get-in test [:checker :checkers :nemesis :checkers])]
    (is (= #{:partition :membership}
           (set (keys checkers))))))

(deftest configures-seed-and-worker-exception-checkers
  (let [checkers (-> (cli/openraft-test {:nemesis :partition
                                         :seed 41
                                         :time-limit 10})
                     :checker
                     :checkers)]
    (testing "the random seed is included in checker results"
      (is (= {:valid? true
              :seed 41}
             (checker/check (:seed checkers)
                            {:seed 41}
                            (history/history [])
                            {}))))

    (testing "unhandled worker exceptions are reported"
      (let [op {:type :info
                :f :read
                :exception {:via [{:type 'java.lang.IllegalStateException}]}}
            result (checker/check (:exceptions checkers)
                                  {}
                                  (history/history [op])
                                  {})
            exception (first (:exceptions result))]
        (is (= :unknown (:valid? result)))
        (is (= {:count 1
                :class 'java.lang.IllegalStateException}
               (select-keys exception [:count :class])))
        (is (true? (:valid? (checker/check (:exceptions checkers)
                                           {}
                                           (history/history [])
                                           {}))))))))

(deftest checks-downloaded-logs-for-node-crashes
  (let [^java.io.File temp-dir
        (.toFile
         (java.nio.file.Files/createTempDirectory
          "openraft-jepsen-checker"
          (make-array java.nio.file.attribute.FileAttribute 0)))
        test {:name "crash-checker-test"
              :start-time "run"
              :nodes ["n1" "n2"]}
        crash (-> (cli/openraft-test {:nemesis :partition
                                      :time-limit 10})
                  :checker
                  :checkers
                  :crash)]
    (try
      (with-redefs [store/base-dir (.getPath temp-dir)]
        (let [write-log! (fn [node content]
                           (let [file (store/path test node "openraft.log")]
                             (io/make-parents file)
                             (spit file content)))
              check #(checker/check crash test (history/history []) {})]
          (write-log! "n1" "INFO node started\n")
          (write-log! "n2" "INFO vote committed\n")
          (testing "clean logs pass"
            (is (= {:valid? true
                    :count 0
                    :missing-nodes []}
                   (select-keys (check)
                                [:valid? :count :missing-nodes]))))

          (write-log! "n2" "INFO before panic\nOPENRAFT_JEPSEN_PANIC\n")
          (testing "a panic marker fails the check"
            (is (= {:valid? false
                    :count 1
                    :matches [{:node "n2"
                               :line "OPENRAFT_JEPSEN_PANIC"}]}
                   (select-keys (check) [:valid? :count :matches]))))

          (write-log! "n2" "fatal runtime error: stack overflow\n")
          (testing "a fatal runtime error fails the check"
            (is (= {:valid? false
                    :count 1
                    :matches [{:node "n2"
                               :line "fatal runtime error: stack overflow"}]}
                   (select-keys (check) [:valid? :count :matches]))))

          (io/delete-file (store/path test "n2" "openraft.log"))
          (testing "a missing node log is indeterminate"
            (is (= {:valid? :unknown
                    :count 0
                    :missing-nodes ["n2"]}
                   (select-keys (check)
                                [:valid? :count :missing-nodes]))))))
      (finally
        (doseq [file (reverse (file-seq temp-dir))]
          (io/delete-file file true))))))

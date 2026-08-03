(ns jepsen.openraft.checker-test
  (:require [clojure.java.io :as io]
            [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.history :as history]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.store :as store]))

(deftest records-the-random-seed
  (is (= {:valid? true
          :seed 41}
         (checker/check (openraft-checker/random-seed-checker)
                        {:seed 41}
                        (history/history [])
                        {}))))

(deftest reports-unhandled-worker-exceptions
  (let [subject (openraft-checker/strict-unhandled-exceptions)
        op {:type :info
            :f :read
            :exception {:via [{:type 'java.lang.IllegalStateException}]}}
        result (checker/check subject {} (history/history [op]) {})
        exception (first (:exceptions result))]
    (is (= :unknown (:valid? result)))
    (is (= {:count 1
            :class 'java.lang.IllegalStateException}
           (select-keys exception [:count :class])))
    (is (true? (:valid? (checker/check subject
                                       {}
                                       (history/history [])
                                       {}))))))

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

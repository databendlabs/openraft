(ns jepsen.openraft.cli-test
  (:require [clojure.test :refer [deftest is testing]]
            [clojure.tools.cli :as tools-cli]
            [jepsen.openraft.cli :as cli]
            [jepsen.openraft.db :as openraft-db]
            [jepsen.random :as random]))

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

(ns jepsen.openraft.cli-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.history :as history]
            [jepsen.openraft.cli :as cli]
            [jepsen.random :as random]))

(deftest records-and-applies-the-random-seed
  (testing "a seed is generated when none is supplied"
    (let [seed (get-in (#'cli/ensure-random-seed {:options {}})
                       [:options :seed])]
      (is (integer? seed))))

  (testing "an explicit seed is preserved, stored, and applied"
    (let [applied-seed (atom nil)
          parsed (#'cli/ensure-random-seed
                  {:options {:nemesis :partition
                             :seed 41
                             :time-limit 10}})]
      (with-redefs [random/set-seed! #(reset! applied-seed %)]
        (let [test (cli/openraft-test (:options parsed))]
          (is (= 41 (:seed test)))
          (is (= 41 @applied-seed)))))))

(deftest configures-worker-exception-and-node-crash-checkers
  (let [checkers (-> (cli/openraft-test {:nemesis :partition
                                         :time-limit 10})
                     :checker
                     :checkers)]
    (testing "unhandled worker exceptions are reported"
      (let [op {:type :info
                :f :read
                :exception {:via [{:type 'java.lang.IllegalStateException}]}}
            result (checker/check (:exceptions checkers)
                                  {}
                                  (history/history [op])
                                  {})
            exception (first (:exceptions result))]
        (is (true? (:valid? result)))
        (is (= {:count 1
                :class 'java.lang.IllegalStateException}
               (select-keys exception [:count :class])))))

    (testing "node panic and fatal messages are matched"
      (let [{:keys [filename pattern]} (:crash checkers)]
        (is (= "openraft.log" filename))
        (is (re-find pattern
                     "thread 'main' panicked at src/main.rs:10:5"))
        (is (re-find pattern "fatal runtime error: stack overflow"))
        (is (not (re-find pattern "INFO openraft::raft: vote committed")))))))

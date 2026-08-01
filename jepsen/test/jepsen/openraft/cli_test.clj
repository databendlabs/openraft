(ns jepsen.openraft.cli-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.history :as history]
            [jepsen.openraft.cli :as cli]))

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

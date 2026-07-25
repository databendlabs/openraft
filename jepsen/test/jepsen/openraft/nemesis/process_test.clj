(ns jepsen.openraft.nemesis.process-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
                    [nemesis :as nemesis]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.process :as process]
            [jepsen.openraft.quorum :as quorum]))

(def voters ["n1" "n2" "n3" "n4" "n5"])

(deftest selects-fault-set-for-leader-mode
  (let [configs [(set voters)]
        fault-sets (set (quorum/fault-sets configs))]
    (doseq [mode [:leader-killed :leader-survives]]
      (testing (name mode)
        (let [targets (#'process/process-targets
                        voters
                        configs
                        "n1"
                        mode)]
          (is (contains? fault-sets (set targets)))
          (is (= (= mode :leader-killed)
                 (contains? (set targets) "n1"))))))))

(deftest restarts-the-processes-that-were-killed
  (let [invocations (atom [])
        delegate (reify nemesis/Nemesis
                   (setup! [this _test]
                     this)
                   (invoke! [_ _test op]
                     (swap! invocations conj op)
                     op)
                   (teardown! [this _test]
                     this))
        subject (process/->ProcessNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"}]
    (with-redefs [cluster/await-ready! (fn [_test] status)
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])]
      (let [killed (nemesis/invoke!
                     subject
                     test
                     {:type :info
                      :f :kill-process
                      :value :leader-killed})
            restarted (nemesis/invoke!
                        subject
                        test
                        {:type :info
                         :f :restart-process})]
        (is (= ["n1"] (get-in killed [:value :nodes])))
        (is (= ["n1"] (get-in restarted [:value :nodes])))
        (is (= [[:kill ["n1"]]
                [:start ["n1"]]]
               (mapv (juxt :f :value) @invocations)))))))

(deftest requires-both-process-modes-and-recovery
  (let [subject (#'process/coverage-checker)
        complete-history [{:f :kill-process
                           :value {:mode :leader-survives}}
                          {:f :kill-process
                           :value {:mode :leader-killed}}
                          {:f :await-recovery
                           :value {:leader "n2"}}]
        incomplete-history [{:f :kill-process
                             :value {:mode :leader-survives}}]]
    (is (:valid? (checker/check subject {} complete-history {})))
    (let [result (checker/check subject {} incomplete-history {})]
      (is (false? (:valid? result)))
      (is (= [:leader-killed] (:missing-modes result)))
      (is (false? (:recovered? result))))))

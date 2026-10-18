(ns jepsen.openraft.replicas-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.replicas :as replicas]))

(def nodes ["n1" "n2" "n3"])

(def test-config
  {:nodes nodes
   :api-port 21001})

(defn- log-id [term index]
  {:leader_id {:term term :node_id "n1"} :index index})

(defn- report [applied & {:as overrides}]
  (merge {:last_applied (log-id 2 applied)
          :applied_chain "00000000000000aa"
          :state_digest "00000000000000bb"
          :key_count 5
          :log_ids (mapv #(log-id 2 %) (range (- applied 2) (inc applied)))}
         overrides))

(deftest finds-divergent-digests-and-log-ids
  (testing "identical replicas have no divergence"
    (is (= [] (replicas/divergences {"n1" (report 10) "n2" (report 10)}))))

  (testing "a different applied chain is reported per node"
    (is (= [{:kind :applied-chain
             :by-node {"n1" "00000000000000aa" "n2" "00000000000000ff"}}]
           (replicas/divergences
            {"n1" (report 10)
             "n2" (report 10 :applied_chain "00000000000000ff")}))))

  (testing "only indexes two replicas retain are compared"
    (let [reports {"n1" (report 10 :log_ids [(log-id 1 7) (log-id 2 8)])
                   "n2" (report 10 :log_ids [(log-id 2 7) (log-id 2 8)
                                             (log-id 2 9)])
                   "n3" (report 10 :log_ids [(log-id 2 10)])}]
      (is (= [{:index 7
               :log-ids {"n1" (log-id 1 7) "n2" (log-id 2 7)}}]
             (replicas/log-id-mismatches reports)))
      (is (= [{:kind :log-ids
               :count 1
               :first [{:index 7
                        :log-ids {"n1" (log-id 1 7) "n2" (log-id 2 7)}}]}]
             (replicas/divergences reports))))))

(defn- responding [responses]
  (let [remaining (atom responses)]
    (fn [endpoint]
      (let [node (subs endpoint 0 2)
            [response & more] (get @remaining node)]
        (when more
          (swap! remaining assoc node more))
        (if (instance? Exception response)
          (throw response)
          response)))))

(deftest waits-for-one-common-applied-log-id
  (with-redefs [client/consistency!
                (responding {"n1" [(report 10)]
                             "n2" [(ex-info "down" {:kind :unreachable})
                                   (report 9)
                                   (report 10)]
                             "n3" [(report 10)]})]
    (let [result (replicas/await-convergence! test-config 5000)]
      (is (true? (:converged? result)))
      (is (= (log-id 2 10) (:last-applied result)))
      (is (= [] (:divergences result)))
      (is (= #{"n1" "n2" "n3"} (set (keys (:replicas result)))))
      (is (= 3 (get-in result [:replicas "n2" :retained-log-ids]))))))

(deftest reports-replicas-that-never-converge
  (with-redefs [client/consistency!
                (responding {"n1" [(report 10)]
                             "n2" [(report 8)]
                             "n3" [(ex-info "down" {:kind :unreachable})]})]
    (let [result (replicas/await-convergence! test-config 300)]
      (is (false? (:converged? result)))
      (is (not (contains? result :divergences)))
      (is (= {"n1" (log-id 2 10) "n2" (log-id 2 8) "n3" nil}
             (into {} (map (fn [[node summary]]
                             [node (:last-applied summary)]))
                   (:replicas result)))))))

(deftest propagates-harness-errors
  (with-redefs [client/consistency!
                (fn [_] (throw (IllegalStateException. "harness bug")))]
    (is (thrown? IllegalStateException
                 (replicas/await-convergence! test-config 300)))))

(deftest the-nemesis-records-the-comparison
  (with-redefs [client/consistency! (constantly (report 10))]
    (let [op (nemesis/invoke! (:nemesis (replicas/package))
                              test-config
                              replicas/converge-op)]
      (is (= :converge-replicas (:f op)))
      (is (true? (get-in op [:value :converged?]))))))

(deftest checks-the-recorded-comparison
  (let [check (fn [& values]
                (checker/check (replicas/checker)
                               test-config
                               (into [(assoc replicas/converge-op
                                             :process :nemesis)]
                                     (map #(assoc replicas/converge-op
                                                  :process :nemesis
                                                  :value %))
                                     values)
                               {}))]
    (is (= :unknown (:valid? (check))) "a missing comparison is not a pass")
    (is (true? (:valid? (check {:converged? true :divergences []}))))
    (is (= {:valid? false
            :reason :replicas-did-not-converge
            :replicas {"n1" nil}}
           (check {:converged? false :replicas {"n1" nil}})))
    (is (= :replicas-diverged
           (:reason (check {:converged? true
                            :divergences [{:kind :state-digest}]}))))))
